#!/usr/bin/env python3
"""Append a correctly-numbered item to a process-improvement ledger.

Detects the ledger's own heading convention (`### NNN — title` in the UI
ledger, `## N. title` in the tint ledger, plain `###` otherwise), computes
the next item number, and appends a dated entry with the body read from
stdin. Exists so arc close-outs never mis-number or mis-format an entry.

Usage:
  uv run python scripts/ledger_append.py <ledger.md> --title "..." [--date YYYY-MM-DD] < body.md
"""
import argparse
import datetime
import re
import sys
from pathlib import Path


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("ledger")
    ap.add_argument("--title", required=True)
    ap.add_argument("--date", default=datetime.date.today().isoformat())
    args = ap.parse_args()

    path = Path(args.ledger)
    if not path.exists():
        sys.exit(f"ERROR: {path} does not exist")
    text = path.read_text(encoding="utf-8")

    em_dash = re.findall(r"^### (\d+) — ", text, re.M)
    dotted = re.findall(r"^## (\d+)\. ", text, re.M)
    body = sys.stdin.read().strip()
    if em_dash:
        n = max(int(x) for x in em_dash) + 1
        heading = f"### {n} — {args.title}"
    elif dotted:
        n = max(int(x) for x in dotted) + 1
        heading = f"## {n}. {args.title}"
    else:
        heading = f"### {args.title}"
    entry = f"\n{heading}\n\n_{args.date}_\n\n{body}\n"
    with path.open("a", encoding="utf-8") as f:
        f.write(entry)
    print(f"APPENDED: {heading!r} to {path}")


if __name__ == "__main__":
    main()
