#!/usr/bin/env python3
"""Parse-don't-eyeball audit of tint-palette assignment in a decomposed export.

The tint-palette workflow (docs/tint-palette-workflow.md §5.1) says: PARSE the
exported `palettes.json` plus every object's `palette_id` — never eyeball one
object and conclude the field is right. This is that parse.

Usage:
    uv run python scripts/tint_audit.py <export_dir>     # export root or a package dir
    uv run python scripts/tint_audit.py --selftest

Reports, per package (`Packages/<name>/palettes.json` + `scene.json`):
  * per-palette object counts, including palettes referenced by nobody
  * objects whose `palette_id` resolves to no entry in palettes.json
    (unresolvable — a real bug: the id→palette resolve is broken)
  * ALL_SAME: every object carrying the identical id. Ledger item 1 — the
    classic signature of reading the WRONG per-object field (the +170 flags
    word instead of the +172 index word), which makes every object collapse
    onto one palette.
  * NO_OVERRIDE: null / missing / sentinel (0, 0xFFFF) ids. Ledger item 2 —
    this is NOT a bug: no override means the scene default palette applies.

An object is any JSON object anywhere in `scene.json` carrying a `palette_id`
key (root entity, children, interiors, interior placements) — discovered
structurally, so no key path or asset name is hard-coded.

Exit status: 0 clean, 1 unresolvable ids and/or an ALL_SAME signal, 2 usage.
Dependency-free: stdlib only.
"""

import json
import sys
from collections import Counter
from pathlib import Path

# Sentinel per-object indices meaning "no override" (docs/tint-palette-workflow.md §3).
NO_OVERRIDE_SENTINELS = (0, 0xFFFF)


def collect_palette_ids(node, out):
    """Recursively count every `palette_id` value in a decoded scene document."""
    if isinstance(node, dict):
        if "palette_id" in node:
            value = node["palette_id"]
            out[value if isinstance(value, (str, int)) or value is None else repr(value)] += 1
        for value in node.values():
            collect_palette_ids(value, out)
    elif isinstance(node, list):
        for value in node:
            collect_palette_ids(value, out)
    return out


def audit_package(palettes_path):
    """Audit one package dir. Returns (lines, problem_count)."""
    pkg = palettes_path.parent
    scene_path = pkg / "scene.json"
    lines = [f"package={pkg.name}"]

    declared = json.loads(palettes_path.read_text())
    entries = declared["palettes"] if isinstance(declared, dict) else declared
    declared_ids = [e.get("id") for e in entries]
    lines.append(f"palettes_declared={len(declared_ids)}")
    duplicates = [i for i, n in Counter(declared_ids).items() if n > 1]
    if duplicates:
        lines.append(f"palettes_duplicate_ids={len(duplicates)} {sorted(duplicates)}")

    if not scene_path.exists():
        lines.append(f"scene_json=MISSING {scene_path}")
        return lines, 1

    counts = collect_palette_ids(json.loads(scene_path.read_text()), Counter())

    no_override = sum(n for v, n in counts.items() if v is None or v in NO_OVERRIDE_SENTINELS)
    assigned = {v: n for v, n in counts.items() if v is not None and v not in NO_OVERRIDE_SENTINELS}
    total = sum(counts.values())

    lines.append(f"objects_with_palette_field={total}")
    lines.append(f"objects_assigned={sum(assigned.values())}")
    lines.append(f"objects_no_override={no_override}  # scene default, NOT a bug (ledger 2)")
    lines.append(f"distinct_assigned_palettes={len(assigned)}")

    lines.append("")
    lines.append("count  status        palette_id")
    known = set(declared_ids)
    for pid, n in sorted(assigned.items(), key=lambda kv: (-kv[1], str(kv[0]))):
        lines.append(f"{n:<6} {'ok' if pid in known else 'UNRESOLVABLE':<13} {pid}")
    for pid in sorted(i for i in known if i not in assigned):
        lines.append(f"{0:<6} {'unreferenced':<13} {pid}")

    unresolvable = {p: n for p, n in assigned.items() if p not in known}
    problems = 0
    lines.append("")
    if unresolvable:
        problems += 1
        lines.append(
            f"UNRESOLVABLE_IDS={len(unresolvable)} objects={sum(unresolvable.values())}"
            "  # id->palette resolve is broken"
        )
    else:
        lines.append("UNRESOLVABLE_IDS=0")

    if len(assigned) == 1 and sum(assigned.values()) > 1:
        problems += 1
        lines.append(
            f"ALL_SAME=SUSPICIOUS objects={sum(assigned.values())}"
            "  # every object one palette -> likely wrong per-object field (ledger 1)"
        )
    else:
        lines.append("ALL_SAME=no")
    return lines, problems


def main(argv):
    if len(argv) != 1:
        print(__doc__.strip().splitlines()[2], file=sys.stderr)
        print("usage: tint_audit.py <export_dir> | --selftest", file=sys.stderr)
        return 2
    if argv[0] == "--selftest":
        return selftest()

    root = Path(argv[0]).expanduser()
    if not root.is_dir():
        print(f"not a directory: {root}", file=sys.stderr)
        return 2
    found = sorted(root.rglob("palettes.json"))
    if not found:
        print(f"no palettes.json under {root}", file=sys.stderr)
        return 2

    problems = 0
    for i, path in enumerate(found):
        if i:
            print()
        lines, n = audit_package(path)
        problems += n
        print("\n".join(lines))
    print()
    print(f"packages={len(found)} problem_signals={problems}")
    return 1 if problems else 0


def selftest():
    """Synthetic exports exercising each signal. Values are obviously fake."""
    import tempfile

    def write(dirpath, declared, scene):
        pkg = Path(dirpath)
        (pkg / "palettes.json").write_text(json.dumps({"palettes": declared, "version": 1}))
        (pkg / "scene.json").write_text(json.dumps(scene))
        return pkg / "palettes.json"

    with tempfile.TemporaryDirectory() as tmp:
        # Healthy: two palettes in use, one no-override, all ids resolvable.
        lines, problems = audit_package(
            write(
                tmp,
                [{"id": "palette/synthetic-a"}, {"id": "palette/synthetic-b"}],
                {"children": [{"palette_id": "palette/synthetic-a"},
                              {"palette_id": "palette/synthetic-b"},
                              {"palette_id": None},
                              {"interiors": [{"placements": [{"palette_id": 0xFFFF}]}]}]},
            ),
            )
        text = "\n".join(lines)
        assert problems == 0, text
        assert "objects_no_override=2" in text, text
        assert "ALL_SAME=no" in text, text
        assert "distinct_assigned_palettes=2" in text, text

    with tempfile.TemporaryDirectory() as tmp:
        # Wrong-field signature: every object collapsed onto one palette, and
        # one id that palettes.json never declares.
        lines, problems = audit_package(
            write(
                tmp,
                [{"id": "palette/synthetic-a"}],
                {"children": [{"palette_id": "palette/synthetic-a"},
                              {"palette_id": "palette/synthetic-a"}]},
            ),
            )
        assert problems == 1 and "ALL_SAME=SUSPICIOUS" in "\n".join(lines)

    with tempfile.TemporaryDirectory() as tmp:
        lines, problems = audit_package(
            write(
                tmp,
                [{"id": "palette/synthetic-a"}],
                {"children": [{"palette_id": "palette/synthetic-a"},
                              {"palette_id": "palette/synthetic-missing"}]},
            ),
            )
        text = "\n".join(lines)
        assert problems == 1 and "UNRESOLVABLE_IDS=1" in text, text
        assert "unreferenced" not in text, text

    print("selftest=OK")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
