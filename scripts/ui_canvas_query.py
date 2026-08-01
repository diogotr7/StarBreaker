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

`node` and `entries` also take projection flags, so the raw JSON never has
to be piped through an ad-hoc filter:

  --fields a.b.c,d.e   print one `path=value` line per dotted path
                       (absent paths print `path=<absent>`), replacing the
                       summary/raw dump
  --filter KEY=VALUE   keep only results whose dotted KEY equals VALUE
                       (`entries` keeps its legacy name-substring meaning
                       when the argument contains no `=`)
"""
import argparse
import json
import sys
from pathlib import Path

from ui_ir_query import match_filter, parse_fields, project  # sibling script, same dir

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
    hits = [n for n in hits if match_filter(n, args.filter)]
    if not hits:
        sys.exit(f"error: no node named {args.name!r}")
    fields = parse_fields(args.fields)
    for node in hits:
        if fields:
            print("\n".join(project(node, fields)))
            continue
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
    fields = parse_fields(args.fields)
    dotted = args.filter and "=" in args.filter
    shown = 0
    for e in entries:
        name = e.get("name", "?")
        if dotted:
            if not match_filter(e, args.filter):
                continue
        elif args.filter and args.filter.lower() not in name.lower():
            continue
        shown += 1
        if fields:
            print(f"=== {name}")
            print("\n".join("  " + part for part in project(e, fields)))
            continue
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
    fields_help = "comma-separated dotted paths to project (one path=value line each)"
    p = sub.add_parser("node")
    p.add_argument("record")
    p.add_argument("name")
    p.add_argument("--raw", action="store_true")
    p.add_argument("--fields", help=fields_help)
    p.add_argument("--filter", metavar="KEY=VALUE",
                   help="keep only nodes whose dotted KEY equals VALUE")
    p = sub.add_parser("entries")
    p.add_argument("record")
    p.add_argument("--filter", metavar="SUBSTR|KEY=VALUE",
                   help="name substring, or dotted KEY=VALUE equality")
    p.add_argument("--fields", help=fields_help)
    p = sub.add_parser("tag")
    p.add_argument("query")
    args = ap.parse_args()
    tags = load_tag_names()
    {"node": cmd_node, "entries": cmd_entries, "tag": cmd_tag}[args.cmd](args, tags)


if __name__ == "__main__":
    main()
