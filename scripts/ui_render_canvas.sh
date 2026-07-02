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
