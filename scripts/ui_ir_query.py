#!/usr/bin/env python3
"""Query composed UI IR JSON (plan P1.1, ledger items 19/23).

Input: an IR document produced by `ui render --dump-ir-dir <dir>` (one
`*.ir.json` per helper; see docs/ui-reference.md §6).

Every subcommand takes `--fields a.b,c` (append dotted-path lookups into
each node's JSON as `path=value`, absent paths as `path=<absent>`) and
`--filter KEY=VALUE` (print only nodes whose dotted KEY equals VALUE) —
so no `| python3 -c '<json filter>'` pipe is needed, e.g.
`--filter name=ComponentRoot --fields computed_rect,text_style.font_size`.

Subcommands:
  query <ir.json> <regex>
      One line per node whose `name` OR `text_payload.text` matches the
      regex (re.search, case-sensitive). Always prints id, parent,
      node_type, name, computed_rect, is_active.
  tree <ir.json> <node_id>
      Ancestor chain (root first, indented) for one node: computed_rect,
      authored_size, anchor/pivot, padding, margin.
  children <ir.json> <node_id> [--depth N]
      Descendant subtree (indented by depth) for one node: id, name,
      node_type, x/y/w/h, right (x+w), is_active, and a non-Visible
      overflow mode — the mirror of `tree`, for clip/overflow tracing.
      `--filter` hides non-matching rows but still walks their children.

Dependency-free: stdlib json/re/argparse only.
"""

import argparse
import json
import re
import sys


def load_nodes(path):
    with open(path) as handle:
        doc = json.load(handle)
    nodes = doc.get("nodes")
    if not isinstance(nodes, list):
        sys.exit(f"error: {path} has no 'nodes' array — is it an IR JSON from --dump-ir-dir?")
    return doc, {node["id"]: node for node in nodes}, nodes


def fmt_rect(rect):
    if not isinstance(rect, dict):
        return "none"
    return "({x:g},{y:g},{w:g},{h:g})".format(
        x=rect.get("x", 0.0), y=rect.get("y", 0.0),
        w=rect.get("w", 0.0), h=rect.get("h", 0.0),
    )


MISSING = object()  # distinguishes "path absent" from "path present but null"


def lookup_path(node, dotted, default=None):
    value = node
    for part in dotted.split("."):
        if isinstance(value, dict) and part in value:
            value = value[part]
        elif isinstance(value, list) and part.isdigit() and int(part) < len(value):
            value = value[int(part)]
        else:
            return default
    return value


def parse_fields(spec):
    """'a.b,c' -> ['a.b', 'c']; None/'' -> []."""
    return [field for field in (spec or "").split(",") if field]


def project(obj, fields):
    """Dotted-path projection: ['a.b=1', 'c=<absent>'] (absent != null)."""
    out = []
    for field in fields:
        value = lookup_path(obj, field, MISSING)
        out.append(field + "=" + ("<absent>" if value is MISSING else json.dumps(value)))
    return out


def match_filter(obj, spec):
    """`--filter key=value`: dotted-path equality against raw or JSON form."""
    if not spec:
        return True
    key, _, want = spec.partition("=")
    value = lookup_path(obj, key, MISSING)
    if value is MISSING:
        return False
    return str(value) == want or json.dumps(value) == want


def cmd_query(args):
    _, _, nodes = load_nodes(args.ir_json)
    pattern = re.compile(args.regex)
    fields = parse_fields(args.fields)
    matched = 0
    for node in nodes:
        name = node.get("name") or ""
        text = ((node.get("text_payload") or {}).get("text")) or ""
        if not (pattern.search(name) or pattern.search(text)):
            continue
        if not match_filter(node, args.filter):
            continue
        matched += 1
        row = (
            f"id={node.get('id')} parent={node.get('parent_id')} "
            f"type={node.get('node_type')} active={node.get('is_active')} "
            f"rect={fmt_rect(node.get('computed_rect'))} name={name!r}"
        )
        row += "".join(" " + part for part in project(node, fields))
        print(row)
    if matched == 0:
        print(f"no nodes matched {args.regex!r} (searched name + text_payload.text)",
              file=sys.stderr)
        return 1
    return 0


def cmd_tree(args):
    _, by_id, _ = load_nodes(args.ir_json)
    node = by_id.get(args.node_id)
    if node is None:
        sys.exit(f"error: node id {args.node_id} not in {args.ir_json}")
    chain = [node]
    seen = {args.node_id}
    while chain[0].get("parent_id") is not None:
        parent_id = chain[0]["parent_id"]
        if parent_id in seen or parent_id not in by_id:
            break  # cycle or dangling parent: stop rather than loop
        seen.add(parent_id)
        chain.insert(0, by_id[parent_id])
    fields = parse_fields(args.fields)
    for depth, entry in enumerate(chain):
        if not match_filter(entry, args.filter):
            continue
        row = (
            "{indent}id={id} type={ty} name={name!r} rect={rect} "
            "authored_size={size} anchor={anchor} pivot={pivot} "
            "padding={padding} margin={margin}".format(
                indent="  " * depth,
                id=entry.get("id"),
                ty=entry.get("node_type"),
                name=entry.get("name") or "",
                rect=fmt_rect(entry.get("computed_rect")),
                size=json.dumps(entry.get("authored_size")),
                anchor=json.dumps(entry.get("anchor")),
                pivot=json.dumps(entry.get("pivot")),
                padding=json.dumps(entry.get("padding")),
                margin=json.dumps(entry.get("margin")),
            )
        )
        row += "".join(" " + part for part in project(entry, fields))
        print(row)
    return 0


def cmd_children(args):
    _, by_id, nodes = load_nodes(args.ir_json)
    if args.node_id not in by_id:
        sys.exit(f"error: node id {args.node_id} not in {args.ir_json}")
    children_of = {}
    for node in nodes:
        children_of.setdefault(node.get("parent_id"), []).append(node)
    fields = parse_fields(args.fields)

    def overflow_mode(node):
        mode = node.get("overflow_mode")
        return mode.get("overflow") if isinstance(mode, dict) else None

    def walk(node, depth):
        rect = node.get("computed_rect") or {}
        x = rect.get("x", 0.0)
        w = rect.get("w", 0.0)
        row = (
            "{indent}id={id} {name!r} {ty} x={x:g} y={y:g} w={w:g} h={h:g} "
            "right={right:g} active={active}".format(
                indent="  " * depth,
                id=node.get("id"), name=node.get("name") or "",
                ty=node.get("node_type"), x=x, y=rect.get("y", 0.0),
                w=w, h=rect.get("h", 0.0), right=x + w, active=node.get("is_active"),
            )
        )
        mode = overflow_mode(node)
        if mode and mode != "Visible":
            row += f" overflow={mode}"
        row += "".join(" " + part for part in project(node, fields))
        if match_filter(node, args.filter):  # non-matches stay silent but still recurse
            print(row)
        if depth >= args.depth:
            return
        for child in sorted(children_of.get(node.get("id"), []), key=lambda c: c.get("id")):
            walk(child, depth + 1)

    walk(by_id[args.node_id], 0)
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)

    def add_projection(parser_):
        parser_.add_argument("--fields", help="comma-separated dotted paths to also print")
        parser_.add_argument("--filter", metavar="KEY=VALUE",
                             help="keep only nodes whose dotted KEY equals VALUE")

    query = sub.add_parser("query", help="list nodes matching a regex on name or text")
    query.add_argument("ir_json")
    query.add_argument("regex")
    add_projection(query)
    query.set_defaults(func=cmd_query)

    tree = sub.add_parser("tree", help="ancestor chain with layout fields for one node")
    tree.add_argument("ir_json")
    tree.add_argument("node_id", type=int)
    add_projection(tree)
    tree.set_defaults(func=cmd_tree)

    children = sub.add_parser("children", help="descendant subtree with rect/overflow for one node")
    children.add_argument("ir_json")
    children.add_argument("node_id", type=int)
    children.add_argument("--depth", type=int, default=6, help="max descendant depth (default 6)")
    add_projection(children)
    children.set_defaults(func=cmd_children)

    args = parser.parse_args()
    sys.exit(args.func(args))


if __name__ == "__main__":
    main()
