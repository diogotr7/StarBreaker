#!/usr/bin/env bash
# Decomposed ship export wrapper with a stale-binary guard.
#
# Transcripts record repeated incidents of exporting (and then benchmarking or
# adjudicating a fix against) a `target/release/starbreaker` built before the
# commit under test. This wrapper refuses to run when the binary is older than
# the newest commit touching `cli/` or `crates/`.
#
# Usage:
#   bash scripts/export_ship.sh <entity> [outdir] [--lod N] [--mip N]
#
#   outdir defaults to "$PWD/../ships" when that directory exists; otherwise it
#   is required (no workspace-specific path is baked in — AGENTS.md).
#   --lod / --mip also read the LOD / MIP env vars; both default to 0.
#
# SC_DATA_P4K is deliberately NOT set — the CLI auto-detects the data path.
# Final line is a machine-readable marker: `EXPORT: OK <dir>` or `EXPORT: FAILED`.
set -uo pipefail

REPO_ROOT="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

ENTITY=""
OUTDIR=""
LOD="${LOD:-0}"
MIP="${MIP:-0}"

while [ $# -gt 0 ]; do
    case "$1" in
        --lod) LOD="$2"; shift 2 ;;
        --mip) MIP="$2"; shift 2 ;;
        -h|--help) sed -n '2,18p' "${BASH_SOURCE[0]}"; exit 0 ;;
        -*) echo "unknown option: $1" >&2; exit 2 ;;
        *)
            if [ -z "$ENTITY" ]; then ENTITY="$1"
            elif [ -z "$OUTDIR" ]; then OUTDIR="$1"
            else echo "unexpected argument: $1" >&2; exit 2
            fi
            shift ;;
    esac
done

if [ -z "$ENTITY" ]; then
    echo "usage: export_ship.sh <entity> [outdir] [--lod N] [--mip N]" >&2
    exit 2
fi

if [ -z "$OUTDIR" ]; then
    if [ -d "$PWD/../ships" ]; then
        OUTDIR="$(cd -- "$PWD/../ships" && pwd)"
    else
        echo "no outdir given and \$PWD/../ships does not exist — pass one explicitly" >&2
        exit 2
    fi
fi

BIN="$REPO_ROOT/target/release/starbreaker"
NEWEST_COMMIT="$(git -C "$REPO_ROOT" log -1 --format=%ct -- cli crates 2>/dev/null || true)"
BIN_MTIME=0
[ -x "$BIN" ] && BIN_MTIME="$(stat -c %Y "$BIN")"

if [ ! -x "$BIN" ] || { [ -n "$NEWEST_COMMIT" ] && [ "$BIN_MTIME" -lt "$NEWEST_COMMIT" ]; }; then
    echo "STALE BINARY — run scripts/build-release-cli.sh first"
    exit 1
fi

mkdir -p "$OUTDIR"
echo "entity=$ENTITY outdir=$OUTDIR lod=$LOD mip=$MIP binary_mtime=$BIN_MTIME"

if "$BIN" entity export "$ENTITY" "$OUTDIR" \
        --kind decomposed --lod "$LOD" --mip "$MIP" --materials all; then
    echo "EXPORT: OK $OUTDIR"
else
    echo "EXPORT: FAILED"
    exit 1
fi
