#!/usr/bin/env python3
"""tidy-disk survey: emits the whole survey as JSON on stdout.

Reports only. Never deletes, never writes outside stdout.

Protected-set rule (conservative — unknown means protected):
- every bare `starbreaker-<hash>` binary under target/release/deps (the ladder)
- every ships_perf_bench label the ledger's LATEST "Oracle baselines" /
  "byte-oracle baselines" mention names (parse failure => ALL labels protected)
- ships/ in its entirety (incl. dcb_canvas)
- graphify-out's live set (graph.json, cache/, cost.json, manifest.json)
"""
import json
import os
import re
import subprocess
import sys

# scripts/ -> tidy-disk/ -> skills/ -> .claude/ -> StarBreaker/
REPO = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "..", ".."))
WS = os.path.dirname(REPO)  # workspace root (scorg_tools)


def sh(cmd, timeout=120):
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
        return p.stdout.strip() if p.returncode == 0 else None
    except Exception:
        return None


def du(path):
    if not os.path.exists(path):
        return None
    out = sh(["du", "-sh", "--", path])
    return out.split("\t")[0] if out else None


def survey():
    out = {"repo": REPO, "workspace": WS}
    out["df"] = sh(["df", "-h", REPO])

    # Busy checks — any hit blocks related candidates.
    procs = sh(["ps", "-eo", "comm="]) or ""
    names = set(procs.split())
    out["busy"] = {
        "cargo_or_rustc": bool(names & {"cargo", "rustc"}),
        # starbreaker-mcp is the always-on MCP server; it holds no export dirs.
        "starbreaker_cli": any(
            n.startswith("starbreaker") and n != "starbreaker-mcp" for n in names
        ),
        "blender": "blender" in names,
        "load_1min": float((open("/proc/loadavg").read().split()[0])),
    }

    # target trees
    t = os.path.join(REPO, "target")
    out["target"] = {
        "debug": du(os.path.join(t, "debug")),
        "release": du(os.path.join(t, "release")),
        "perf": du(os.path.join(t, "perf")),
        "tmp_total": du(os.path.join(t, "tmp")),
    }

    # target/tmp: dirs (candidates) vs logs (keep — ledger cites them)
    tmp = os.path.join(t, "tmp")
    dirs, logs = [], []
    if os.path.isdir(tmp):
        for e in sorted(os.listdir(tmp)):
            p = os.path.join(tmp, e)
            if os.path.isdir(p):
                dirs.append({"path": p, "size": du(p)})
            else:
                logs.append(e)
    out["target_tmp"] = {"dirs": dirs, "log_files_keep": logs}

    # Bisect ladder inventory (protected) + stale generations estimate
    deps = os.path.join(t, "release", "deps")
    ladder = []
    if os.path.isdir(deps):
        for e in os.listdir(deps):
            if re.fullmatch(r"starbreaker-[0-9a-f]{16}", e):
                ladder.append(e)
    out["release_deps"] = {
        "ladder_count": len(ladder),
        "ladder_protected": True,
        "size": du(deps),
        "note": "stale .rlib/.rmeta/.so generations proposable (keep newest per crate); NEVER cargo-sweep this dir",
    }

    # ships_perf_bench labels + which the ledger currently names
    bench = os.path.join(WS, "ships_perf_bench")
    ledger = os.path.join(REPO, "docs", "optimisation-ledger.md")
    current = set()
    try:
        text = open(ledger, encoding="utf-8").read()
        # last mention wins: lines like `ships_perf_bench/{carrack,clipper}_aug02`
        for m in re.finditer(r"ships_perf_bench/\{([^}]+)\}_(\w+)", text):
            comps, suffix = m.group(1).split(","), m.group(2)
            current = {f"{c.strip()}_{suffix}" for c in comps}
        for m in re.finditer(r"ships_perf_bench/(\w+)(?:[/\s`]|$)", text):
            pass  # single-label mentions are ambiguous; brace form is the authority
    except OSError:
        current = None  # parse failure => protect everything
    labels = []
    if os.path.isdir(bench):
        for e in sorted(os.listdir(bench)):
            p = os.path.join(bench, e)
            if os.path.isdir(p):
                labels.append({
                    "label": e,
                    "size": du(p),
                    "protected": True if current is None else (e in current),
                })
    out["ships_perf_bench"] = {
        "ledger_current": sorted(current) if current else "UNPARSED — treat all as protected",
        "labels": labels,
    }

    # ships/ — never a candidate; report size + dcb_canvas presence only
    ships = os.path.join(WS, "ships")
    out["ships"] = {
        "size": du(ships),
        "protected": True,
        "dcb_canvas_present": os.path.isdir(os.path.join(ships, "dcb_canvas")),
    }

    # graphify snapshots: live set protected; dated dirs beyond 3 newest proposable
    g = os.path.join(REPO, "graphify-out")
    snaps = []
    if os.path.isdir(g):
        dated = sorted(e for e in os.listdir(g) if re.fullmatch(r"20[0-9-]+", e))
        for i, e in enumerate(dated):
            snaps.append({"snapshot": e, "size": du(os.path.join(g, e)),
                          "protected": i >= len(dated) - 3})
    out["graphify_snapshots"] = snaps
    out["graphify_live_set_present"] = os.path.isfile(os.path.join(g, "graph.json"))

    scratch = sys.argv[1] if len(sys.argv) > 1 else None
    if scratch:
        out["session_scratchpad"] = {"path": scratch, "size": du(scratch)}
    return out


if __name__ == "__main__":
    json.dump(survey(), sys.stdout, indent=2)
    print()
