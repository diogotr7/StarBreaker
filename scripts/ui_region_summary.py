#!/usr/bin/env python3
"""Per-region CHANGED/NEW/same summary for a UI parity loop cycle (plan A3).

Reads a ``cur.json`` produced by ``ui_compare.py --json`` (+ an optional
``prev.json`` from the previous cycle, + the optional reference measurement
bank), and prints one line per region with a flag vs the previous cycle:

  CHANGED  any render mean channel moved > 1.0 (0-255) or any ratio moved > 0.01
  NEW      region absent from prev (the first cycle flags all regions)
  same     within thresholds

RESULT MARKER via atexit: ``ui_region_summary: OK (N regions, M changed)`` /
``ui_region_summary: FAILED (...)`` — never rely on a piped exit code.

Bank annotation: the measurement bank keys entries on their reference
``capture`` path (there is no target_id column), so bank rows for THIS screen
are matched by ``--reference`` (the dossier reference_file) and grouped to a
region by the 2nd dotted component of the entry's ``element`` (e.g.
``power.footer.P-glyph`` -> region ``footer``).

Usage:
  python3 scripts/ui_region_summary.py --cur cur.json [--prev prev.json] \
      [--bank bank.json] [--reference <ref-basename>]
  python3 scripts/ui_region_summary.py --self-test
"""
import argparse
import atexit
import json
import os
import sys

MEAN_THRESH = 1.0    # 0-255 per channel
RATIO_THRESH = 0.01

_STATE = {"marker": "ui_region_summary: FAILED (did not complete)"}


@atexit.register
def _emit_marker():
    print(_STATE["marker"])


def region_changed(cur_r, prev_r):
    """Did the RENDER-side stats move beyond the thresholds? (The reference is
    fixed across cycles; the render is what iterating changes.)"""
    for label in ("bright", "dark"):
        c = cur_r["render"][label]
        p = prev_r["render"][label]
        if (c is None) != (p is None):
            return True
        if c is None:
            continue
        if any(abs(a - b) > MEAN_THRESH for a, b in zip(c["mean"], p["mean"])):
            return True
        if any(abs(a - b) > RATIO_THRESH for a, b in zip(c["ratio"], p["ratio"])):
            return True
    return False


def flag_regions(cur, prev):
    """-> list of (region_dict, flag). Regions absent from prev are NEW."""
    prev_by = {r["region"]: r for r in (prev or {}).get("regions", [])}
    out = []
    for r in cur["regions"]:
        p = prev_by.get(r["region"])
        if p is None:
            out.append((r, "NEW"))
        elif region_changed(r, p):
            out.append((r, "CHANGED"))
        else:
            out.append((r, "same"))
    return out


def bank_by_region(bank, reference):
    """Bank entries for this screen (capture basename == reference), grouped by
    the region token (2nd dotted component of `element`)."""
    if not bank or not reference:
        return {}
    ref_base = os.path.basename(reference)
    groups = {}
    for e in bank.get("entries", []):
        if os.path.basename(e.get("capture", "")) != ref_base:
            continue
        parts = e.get("element", "").split(".")
        region = parts[1] if len(parts) >= 2 else e.get("element", "")
        groups.setdefault(region, []).append(e)
    return groups


def _fmt_side(side):
    def one(s):
        if s is None:
            return "(none)"
        m, ra = s["mean"], s["ratio"]
        return f"mean=({m[0]:.0f},{m[1]:.0f},{m[2]:.0f}) ratio=(1,{ra[1]:.2f},{ra[2]:.2f})"
    return f"bright {one(side['bright'])} | dark {one(side['dark'])}"


def summarise(cur, prev, bank, reference):
    flags = flag_regions(cur, prev)
    banks = bank_by_region(bank, reference)
    changed = 0
    for r, flag in flags:
        if flag in ("CHANGED", "NEW"):
            changed += 1
        print(f"[{r['region']:<14}] {flag:<8} render {_fmt_side(r['render'])}")
        if flag != "same":
            print(f"    crop: {r.get('crop', '?')}")   # flagged regions only
        for e in banks.get(r["region"], []):
            print(f"    bank: {e['metric']}={e['value']}  ({e['element']})")
    return len(flags), changed


def run_self_test():
    base = {"render": {"bright": {"mean": [180, 70, 0], "ratio": [1.0, 0.39, 0.0], "n": 100},
                       "dark": {"mean": [10, 10, 10], "ratio": [1.0, 1.0, 1.0], "n": 50}}}
    prev = {"regions": [dict(base, region="a"), dict(base, region="b")]}
    a_changed = {"render": {"bright": {"mean": [185, 70, 0], "ratio": [1.0, 0.39, 0.0], "n": 100},
                            "dark": {"mean": [10, 10, 10], "ratio": [1.0, 1.0, 1.0], "n": 50}}}
    cur = {"regions": [dict(a_changed, region="a"), dict(base, region="b")]}
    flags = {r["region"]: f for r, f in flag_regions(cur, prev)}
    assert flags["a"] == "CHANGED", flags   # mean channel 0 moved 180->185 (>1.0)
    assert flags["b"] == "same", flags

    cur2 = {"regions": [dict(base, region="c")]}
    assert {r["region"]: f for r, f in flag_regions(cur2, prev)}["c"] == "NEW"      # absent from prev
    assert {r["region"]: f for r, f in flag_regions(cur2, None)}["c"] == "NEW"      # first cycle

    # ratio-only move must also trip CHANGED
    b_ratio = {"render": {"bright": {"mean": [180, 70, 0], "ratio": [1.0, 0.41, 0.0], "n": 100},
                          "dark": base["render"]["dark"]}}
    cur3 = {"regions": [dict(b_ratio, region="a")]}
    assert {r["region"]: f for r, f in flag_regions(cur3, prev)}["a"] == "CHANGED"  # ratio 0.39->0.41 (>0.01)

    _STATE["marker"] = "ui_region_summary: OK (self-test)"
    return 0


def main(argv):
    ap = argparse.ArgumentParser(description=__doc__,
                                 formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cur", help="cur.json from ui_compare.py --json")
    ap.add_argument("--prev", default=None, help="prev.json (previous cycle)")
    ap.add_argument("--bank", default=None, help="reference measurement bank JSON")
    ap.add_argument("--reference", default=None, help="reference file (bank join key)")
    ap.add_argument("--self-test", action="store_true")
    args = ap.parse_args(argv)

    if args.self_test:
        return run_self_test()
    if not args.cur:
        _STATE["marker"] = "ui_region_summary: FAILED (--cur required)"
        return 2
    with open(args.cur, encoding="utf-8") as f:
        cur = json.load(f)
    prev = None
    if args.prev and os.path.isfile(args.prev):
        with open(args.prev, encoding="utf-8") as f:
            prev = json.load(f)
    bank = None
    if args.bank and os.path.isfile(args.bank):
        with open(args.bank, encoding="utf-8") as f:
            bank = json.load(f)
    n, m = summarise(cur, prev, bank, args.reference)
    _STATE["marker"] = f"ui_region_summary: OK ({n} regions, {m} changed)"
    return 0


if __name__ == "__main__":
    try:
        rc = main(sys.argv[1:])
    except Exception as e:
        _STATE["marker"] = f"ui_region_summary: FAILED ({type(e).__name__}: {e})"
        rc = 1
    sys.exit(rc)
