# UI Retro Recommendations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement the Carrack-arc retrospective's outstanding "FIX it, don't just note it" items: a standalone canvas-render harness, a structural canvas/kit-sheet/tag query tool, draw-layer test assertions, docs, and the skill-file updates recorded in the two `recommendations.md` files.

**Architecture:** Two new dev tools in `StarBreaker/scripts/` following the existing `ui_render.sh`/`ui_compare.py` conventions; two strengthened tests in the existing expansion test module; doc entries in `ui-reference.md`; skill text edits applied to the canonical skill files.

**Tech Stack:** bash, Python 3 (stdlib only), Rust test edits, markdown.

## Global Constraints

- No maintainer names anywhere; no `/home/<user>` paths — scripts use `$HOME`/repo-root-relative resolution (`StarBreaker/AGENTS.md`).
- No hard-coded game-data VALUES; tools read live decompiled records.
- Ledger discipline: one commit per coherent item citing its ledger number (101/103); `bash scripts/ui_check.sh` result marker read per commit (grep the marker, not the exit code).
- Process changes must not alter render behaviour (only Task 3 touches crate code, tests only).
- `SC_DATA_P4K` is required by `ui render`; scripts must say so in their header comment.

---

### Task 1: `scripts/ui_canvas_query.py` — structural record/tag query tool

**Files:**
- Create: `StarBreaker/scripts/ui_canvas_query.py`

**Interfaces:**
- Consumes: decompiled records under `<workspace>/ships/dcb_canvas/libs/foundry/records/`; the tag database `tagdatabase/tagdatabase.tagdatabase.json`.
- Produces: CLI with subcommands `node`, `entries`, `tag` (used by Task 4 docs).

- [ ] **Step 1: Write the script**

```python
#!/usr/bin/env python3
"""Structural queries over decompiled BuildingBlocks records + the tag database.

Parse JSON by STRUCTURE, never line-grep a big nested record (ledger 68).
Born from the Carrack lift-call arc (ledger 101/103), where these three
lookups were each retyped as ad-hoc one-liners many times.

Usage (paths may be absolute or relative to the workspace record root
<workspace>/ships/dcb_canvas/libs/foundry/records):

  python3 scripts/ui_canvas_query.py node <record.json> <node-name> [--raw]
      Dump a scene node's authored fields (type, renderer, styleTags with
      tag names resolved, icon/label properties, fillStyle, svgFill, canvas).

  python3 scripts/ui_canvas_query.py entries <record.json> [--filter SUBSTR]
      Dump a style record's / kit sheet's entries: name, conditions (tag
      UUIDs resolved to names, Ancestor/Parent structure summarised) and
      modifiers (field = value; ColorStyle roles and ColorSolid RGBA shown).

  python3 scripts/ui_canvas_query.py tag <uuid-or-name-substring>
      Resolve tag uuid -> name, or list tags whose name contains the text.
"""
import argparse
import json
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
RECORD_ROOT = REPO_ROOT.parent / "ships/dcb_canvas/libs/foundry/records"
TAGDB = RECORD_ROOT / "tagdatabase/tagdatabase.tagdatabase.json"


def resolve_record_path(arg: str) -> Path:
    p = Path(arg)
    if p.is_file():
        return p
    candidate = RECORD_ROOT / arg
    if candidate.is_file():
        return candidate
    sys.exit(f"error: record not found: {arg} (tried {candidate})")


def load_tag_names() -> dict:
    names = {}

    def walk(o):
        if isinstance(o, dict):
            if o.get("_Type_") == "Tag" and "tagName" in o:
                names[o.get("_RecordId_", "")] = o["tagName"]
            for v in o.values():
                walk(v)
        elif isinstance(o, list):
            for v in o:
                walk(v)

    walk(json.load(open(TAGDB)))
    return names


def tag_name(tags: dict, uuid: str) -> str:
    return tags.get(uuid, uuid[:8] + "?")


def find_nodes(obj, name, hits):
    if isinstance(obj, dict):
        if obj.get("name") == name:
            hits.append(obj)
        for v in obj.values():
            find_nodes(v, name, hits)
    elif isinstance(obj, list):
        for v in obj:
            find_nodes(v, name, hits)


NODE_FIELDS = [
    "_Type_", "rendererType", "isActive", "instantiated", "fillStyle",
    "iconPosition", "canvas", "urlPostfix", "urlOptional",
]
NODE_NESTED = ["iconProperties", "labelProperties", "svgFill", "background"]


def cmd_node(args, tags):
    record = json.load(open(resolve_record_path(args.record)))
    hits = []
    find_nodes(record, args.name, hits)
    if not hits:
        sys.exit(f"error: no node named {args.name!r}")
    for node in hits:
        if args.raw:
            print(json.dumps(node, indent=1))
            continue
        print(f"=== {args.name} ===")
        for k in NODE_FIELDS:
            if k in node and node[k] not in (None, "", []):
                print(f"  {k} = {json.dumps(node[k])[:120]}")
        for uuid in [
            (t.get("_RecordId_", "") if isinstance(t, dict) else str(t))
            for t in node.get("styleTags") or []
        ]:
            print(f"  styleTag = {tag_name(tags, uuid)} ({uuid[:8]})")
        for k in NODE_NESTED:
            v = node.get(k)
            if isinstance(v, dict):
                inner = {
                    ik: iv for ik, iv in v.items()
                    if ik != "_Type_" and iv not in (None, "", [], {})
                }
                print(f"  {k} = {json.dumps(inner)[:200]}")


def summarise_condition(cond, tags) -> str:
    ty = cond.get("_Type_", "").replace("BuildingBlocks_StyleSelectorCondition", "")
    tag = (cond.get("tag") or {}).get("_RecordId_", "")
    parts = [ty + (f"({tag_name(tags, tag)})" if tag else "")]
    for key in ("breakConditions", "conditions"):
        inner = cond.get(key)
        if isinstance(inner, list) and inner:
            parts.append(
                key + "[" + ", ".join(summarise_condition(c, tags) for c in inner) + "]"
            )
    anyof = cond.get("tags")
    if isinstance(anyof, list) and anyof:
        names = ", ".join(
            tag_name(tags, (t or {}).get("_RecordId_", "")) for t in anyof
        )
        parts.append(f"anyOf[{names}]")
    return " ".join(parts)


def summarise_modifier(mod, tags) -> str:
    field = mod.get("field", "?")
    if "color" in mod:
        c = mod["color"] or {}
        if c.get("_Type_") == "BuildingBlocks_ColorStyle":
            return f"{field} = ColorStyle:{c.get('color')} a={c.get('alpha')}"
        inner = c.get("color") or {}
        if inner.get("_Type_") == "SRGBA8":
            return (
                f"{field} = ColorSolid rgba({inner.get('r')},{inner.get('g')},"
                f"{inner.get('b')},{inner.get('a')})"
            )
        return f"{field} = {json.dumps(c)[:80]}"
    return f"{field} = {json.dumps(mod.get('value'))[:80]}"


def cmd_entries(args, tags):
    record = json.load(open(resolve_record_path(args.record)))
    rv = record.get("_RecordValue_", record)
    entries = rv.get("entries") or []
    shown = 0
    for e in entries:
        name = e.get("name", "?")
        if args.filter and args.filter.lower() not in name.lower():
            continue
        shown += 1
        conds = []
        for cl in e.get("conditionsList") or []:
            for c in cl.get("conditions") or []:
                conds.append(summarise_condition(c, tags))
        print(f"=== {name}")
        if conds:
            print(f"  when: {' AND '.join(conds)}")
        for m in e.get("modifiers") or []:
            print(f"  {summarise_modifier(m, tags)}")
    print(f"({shown}/{len(entries)} entries shown)")


def cmd_tag(args, tags):
    q = args.query.lower()
    hits = [
        (uuid, name) for uuid, name in tags.items()
        if q in uuid.lower() or q in name.lower()
    ]
    for uuid, name in sorted(hits, key=lambda x: x[1])[:40]:
        print(f"{uuid}  {name}")
    if not hits:
        sys.exit(f"error: no tag matches {args.query!r}")


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)
    p = sub.add_parser("node")
    p.add_argument("record")
    p.add_argument("name")
    p.add_argument("--raw", action="store_true")
    p = sub.add_parser("entries")
    p.add_argument("record")
    p.add_argument("--filter")
    p = sub.add_parser("tag")
    p.add_argument("query")
    args = ap.parse_args()
    tags = load_tag_names()
    {"node": cmd_node, "entries": cmd_entries, "tag": cmd_tag}[args.cmd](args, tags)


if __name__ == "__main__":
    main()
```

- [ ] **Step 2: Verify against real data (all three subcommands)**

Run:
```bash
python3 scripts/ui_canvas_query.py node ui/buildingblocks/props/transitsystem/old_transituipanelexterior_anvl.json Button_Call
python3 scripts/ui_canvas_query.py entries ui/buildingblocks/styles/modularkitstyles/sk_uilo_a/sk_uilo_a_buttonprimarystyles.json --filter Filled
python3 scripts/ui_canvas_query.py tag icon-element
```
Expected: Button_Call shows `_Type_ = "BuildingBlocks_ComponentGeneralButton"`, fillStyle Filled, iconPreset ArrowCaratDoubleUp; entries show `RootFilled…` with `ColorStyle:Background` / `ColorSolid rgba(0,0,0,255)` and tag-named conditions; tag prints `f530a994-… icon-element-instance`.

- [ ] **Step 3: Commit**

```bash
git add scripts/ui_canvas_query.py
git commit -m "tools(ui): ui_canvas_query.py — structural record/kit-sheet/tag queries (ledger 101/103)"
```

---

### Task 2: `scripts/ui_render_canvas.sh` — standalone canvas render harness

**Files:**
- Create: `StarBreaker/scripts/ui_render_canvas.sh`

**Interfaces:**
- Consumes: `starbreaker ui render --scene` (debug binary), `SC_DATA_P4K` env.
- Produces: `bash scripts/ui_render_canvas.sh --canvas <guid> --entity <EntityClassName> [--kind physical|mfd|radar] [--helper name] [--out dir] [--ir]`.

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# Render ONE BuildingBlocks canvas standalone — the ~1s per-canvas iteration
# harness from the Carrack lift-call arc (ledger 101): generates a minimal
# single-binding scene.json and replays it via `ui render --scene`, so no ship
# export or Clipper scene is needed to iterate on a canvas.
#
# Requires SC_DATA_P4K. The --entity class name supplies the manufacturer
# prefix (style selection) and ship-derived UI values.
#
# Usage:
#   bash scripts/ui_render_canvas.sh --canvas <guid> --entity <EntityClassName> \
#       [--kind physical|mfd|radar] [--helper <name>] [--out <dir>] [--ir]
# Example (Carrack lift-call console):
#   bash scripts/ui_render_canvas.sh --canvas a2c5fae4-f018-4d05-8ab7-e4f17a4d8ae4 \
#       --entity ANVL_Carrack --helper console_liftcall
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

CANVAS="" ENTITY="" KIND="physical" HELPER="" OUT="" DUMP_IR=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --canvas) CANVAS="$2"; shift 2 ;;
        --entity) ENTITY="$2"; shift 2 ;;
        --kind) KIND="$2"; shift 2 ;;
        --helper) HELPER="$2"; shift 2 ;;
        --out) OUT="$2"; shift 2 ;;
        --ir) DUMP_IR=1; shift ;;
        -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done
if [[ -z "$CANVAS" || -z "$ENTITY" ]]; then
    echo "error: --canvas and --entity are required (see --help)" >&2
    exit 2
fi
HELPER="${HELPER:-canvas_${CANVAS:0:8}}"
OUT="${OUT:-/tmp/ui_render_canvas/$HELPER}"

SCENE="$(mktemp -t ui_canvas_scene_XXXXXX.json)"
trap 'rm -f "$SCENE"' EXIT
cat > "$SCENE" <<EOF
{
  "root_entity": { "entity_name": "EntityClassDefinition.$ENTITY" },
  "ui_bindings": [
    {
      "binding_kind": "$KIND",
      "source_entity_name": "$HELPER",
      "helper_name": "$HELPER",
      "default_view": "_default",
      "canvas_guid": "$CANVAS"
    }
  ]
}
EOF

echo "==> cargo build (debug)"
cargo build
echo "==> binary: $(stat -c '%y' target/debug/starbreaker | cut -d. -f1)"

rm -rf "$OUT"
mkdir -p "$OUT"
ARGS=(ui render --scene "$SCENE" --out-dir "$OUT")
if [[ "$DUMP_IR" -eq 1 ]]; then
    ARGS+=(--dump-ir-dir "$OUT/ir")
fi
./target/debug/starbreaker "${ARGS[@]}"
ls -1 "$OUT"
# Unique-output discipline (ledger 69): print the md5 so an actually-changed
# render is distinguishable from a viewer-cached "no change" illusion.
for png in "$OUT"/*.png; do
    [ -e "$png" ] && echo "png md5: $(md5sum "$png")"
done
```

- [ ] **Step 2: Verify against the transit console canvas**

Run:
```bash
SC_DATA_P4K="$HOME/Games/star-citizen/drive_c/Program Files/Roberts Space Industries/StarCitizen/LIVE/Data.p4k" \
  bash scripts/ui_render_canvas.sh --canvas a2c5fae4-f018-4d05-8ab7-e4f17a4d8ae4 --entity ANVL_Carrack --helper console_liftcall --ir
```
Expected: `console_liftcall_TEX0.png` written + `ir/console_liftcall_TEX0.ir.json`; md5 printed; PNG matches the arc's final v7 render (`2cf687b1…` md5 — same code, same canvas).

- [ ] **Step 3: Commit**

```bash
git add scripts/ui_render_canvas.sh
git commit -m "tools(ui): ui_render_canvas.sh — standalone single-canvas render harness (ledger 101)"
```

---

### Task 3: Draw-layer assertions in the expansion tests

**Files:**
- Modify: `StarBreaker/crates/starbreaker-ui/src/bb_resolve/engine_parts/engine_04.part` (tests `primary_button_expansion_forwards_host_icon_identity` and `button_expansion_forwards_host_icon_identity_to_icon_instance`)

**Interfaces:**
- Consumes: `BbNode.icon: Option<BbIcon>` (`icon.image_record: Option<String>`), `crate::icon_preset::svg_path_for_preset(&str) -> Option<String>`.

- [ ] **Step 1: Add the baked-layer assertion to the PRIMARY test**

Append inside `primary_button_expansion_forwards_host_icon_identity`, after the existing raw `iconPreset` assertion:

```rust
        // Assert the layer the RENDERER reads: forwarding only `raw` while the
        // parse-time `BbIcon` kept the template default passed the raw-level
        // assertion but drew the wrong glyph (ledger 103).
        let baked = icon_instance
            .icon
            .as_ref()
            .expect("merged icon instance must carry a parse-time BbIcon");
        assert_eq!(
            baked.image_record,
            crate::icon_preset::svg_path_for_preset("ArrowCaratDoubleUp"),
            "the baked BbIcon must resolve the forwarded preset, not the template default"
        );
```

- [ ] **Step 2: Add the equivalent assertion to the SECONDARY test**

Append inside `button_expansion_forwards_host_icon_identity_to_icon_instance`, after the `host_icon_path` assertion:

```rust
        let baked = icon_instance
            .icon
            .as_ref()
            .expect("merged icon instance must carry a parse-time BbIcon");
        assert_eq!(
            baked.image_record,
            crate::icon_preset::svg_path_for_preset("GeneralX"),
            "the baked BbIcon must resolve the forwarded preset, not the template default"
        );
```

- [ ] **Step 3: Run the two tests**

Run: `cargo test -p starbreaker-ui --lib button_expansion -- --nocapture` (matches both)
Expected: PASS (the icon rebuild already landed in `29c2c1fc3`; the assertions now guard it).

- [ ] **Step 4: Prove the guard bites (mutation check, no commit of the mutation)**

Temporarily comment out the `node.icon = Some(crate::bb_scene::parse_icon(...))` line in `forward_host_icon_identity`, re-run the two tests, expect BOTH FAIL on the new assertion; restore the line, re-run, expect PASS.

- [ ] **Step 5: Full suite + commit**

Run: `cargo test -p starbreaker-ui --lib` → expect `test result: ok`.
```bash
git add crates/starbreaker-ui/src/bb_resolve/engine_parts/engine_04.part
git commit -m "test(ui): expansion tests assert the baked BbIcon draw layer (ledger 103)"
```

---

### Task 4: Document the tools + kit-sheet mechanism in ui-reference.md

**Files:**
- Modify: `StarBreaker/crates/starbreaker-ui/docs/ui-reference.md` (commands section near `ui_render.sh`; mechanisms section §4)

- [ ] **Step 1: Add both tools to the commands section (verify-on-write: run each command line as written before committing)**

Insert after the `ui_render.sh` usage block:

```markdown
**Standalone canvas render** (no ship scene needed — ledger 101):
```bash
bash scripts/ui_render_canvas.sh --canvas <guid> --entity <EntityClassName> \
    [--kind physical|mfd|radar] [--helper <name>] [--out <dir>] [--ir]
```
Generates a minimal single-binding scene.json and replays it; prints the PNG
md5 (ledger 69). `--entity` picks the manufacturer/style + ship values.

**Structural record / kit-sheet / tag queries** (parse-by-structure, ledger 68/101/103):
```bash
python3 scripts/ui_canvas_query.py node <record.json> <node-name> [--raw]
python3 scripts/ui_canvas_query.py entries <record.json> [--filter SUBSTR]
python3 scripts/ui_canvas_query.py tag <uuid-or-name-substring>
```
Record paths resolve relative to `ships/dcb_canvas/libs/foundry/records`;
`entries` resolves condition tag UUIDs to names via the tag database.
```

- [ ] **Step 2: Add the modular-kit mechanism note to §4**

Append under the style/cascade mechanisms section:

```markdown
- **Modular-kit component sheets:** a canvas's style link `S_<kit>_<v>` maps to
  `styles/modularkitstyles/sk_<kit>_<v>/sk_<kit>_<v>_<component>styles.json`
  (button primary/secondary, linearprogressmeter, scrollbar — see
  `modular_*_style_path` in `bb_resolve/engine_parts/engine_01.part`). The
  button sheets' state entries (`RootFilled…ElementInstance`) select on the
  widget standards' `icon-/text-element-instance` tags, which the expansion
  attaches to SHOWN instance widgets (engine_04.part; ledger 103). Colour
  modifiers are either `ColorStyle` palette roles or literal `ColorSolid`
  RGBA — a literal application removes any stale `<Field>Token` or the token
  shadows it at draw time.
```

- [ ] **Step 3: Commit**

```bash
git add crates/starbreaker-ui/docs/ui-reference.md
git commit -m "docs(ui): reference entries for ui_render_canvas.sh + ui_canvas_query.py + kit-sheet mechanism (ledger 101/103)"
```

---

### Task 5: Apply the skill-file recommendations (SKILL.md edits)

**Files:**
- Modify: canonical parity skill `SKILL.md` + optimisation skill `SKILL.md`. First check whether `<workspace>/.claude/skills/...` and `StarBreaker/.claude/skills/...` are separate copies (md5) — edit BOTH if separate real files, keeping them identical; commit only the in-repo copies.
- Modify: both `recommendations.md` files — mark the applied entries as folded into SKILL.md (the existing "Cleared the Open rec" pattern).

- [ ] **Step 1: Parity SKILL.md — add the premature-pause red-flag row**

Append to the "Red flags" table:

```markdown
| "That fix landed — a natural checkpoint, I'll ask whether to continue" | A landed sub-fix is NOT an arc boundary. Re-read the confirmed catalog and take the next open item (fully-auto never asks which/whether). If context is genuinely short, SAY so and hand the state to memory/handoff — do not convert budget anxiety into a permission question. |
```

- [ ] **Step 2: Parity SKILL.md — add the perf side-question pointer**

In the autonomous-loop section, after the guard-trips bullet, add:

```markdown
- **Perf side-questions** ("the export got slow") are the
  `starbreaker-optimisation` skill's job — invoke it rather than ad-hoc
  timing. Its first move: pin BOTH baselines' binary provenance
  (`target/release/deps/starbreaker-<hash>` mtimes are a no-rebuild bisect
  ladder) before attributing anything to this arc's changes.
```

- [ ] **Step 3: Optimisation SKILL.md — extend Measurement rules**

Append to the numbered "Measurement rules (hard-won)" list:

```markdown
7. **Validate the baseline's PROVENANCE.** A "fast prior run" may be a STALE
   BINARY from before the regressing commit — stat the binary mtime against
   `git log` before trusting any endpoint. Old hashed executables under
   `target/release/deps/starbreaker-<hash>` are a free no-rebuild
   time-travel bisect ladder.
8. **A silent probe means the WRONG LAYER, not "no cost".** Instrument the
   phase boundary first (per-item heartbeats), then descend — the DDNA
   regression bypassed `cached_load_keyed`, so a `[tex-miss]` probe there
   stayed silent through a 280s phase.
9. **Profilers may be locked down** (`ptrace_scope`, `perf_event_paranoid=4`):
   `/proc/<pid>/task/*/stat` run-state counts (1 R + N S = serial
   main-thread phase) distinguish serial vs parallel phases for free.
10. **Budget /tmp (tmpfs) for bench outputs** — multi-GB export dirs fill it;
    a later run then dies mid-write ("Disk quota exceeded") and poisons the
    comparison. Clean bench dirs between runs.
```

- [ ] **Step 4: Mark the recommendations as folded**

In each `recommendations.md`, annotate the 2026-07-02 entries: `**FOLDED into SKILL.md 2026-07-02**` (keep the text for history, matching the existing cleared-rec style).

- [ ] **Step 5: Commit (in-repo copies only)**

```bash
git add .claude/skills/
git commit -m "skills: fold 2026-07-02 arc recommendations into parity + optimisation SKILL.md"
```

---

## Self-Review

- Spec coverage: harness tool (Task 2), query tool (Task 1), draw-layer tests (Task 3), docs (Task 4), skill updates (Task 5) — all four argument items covered; ledger 101/103 cited in commits.
- Placeholders: none — full script/test/doc text inline.
- Type consistency: `svg_path_for_preset(&str) -> Option<String>` vs `baked.image_record: Option<String>` — `assert_eq!` compares `Option<String>` == `Option<String>` ✓; `icon_instance.icon` is `Option<BbIcon>` ✓.
