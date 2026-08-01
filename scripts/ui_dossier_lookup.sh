#!/usr/bin/env bash
# Screen-dossier lookup: what do we know about a ship's UI screens, and which
# in-game reference captures exist for one of them. Read-only.
#
# Usage:
#   bash scripts/ui_dossier_lookup.sh <ship-folder>            # list its screens
#   bash scripts/ui_dossier_lookup.sh <ship-folder> <screen_id>  # one screen
#
# Everything comes from crates/starbreaker-ui/data/ui_screen_dossier_v1.json
# and the reference tree; nothing about any ship/screen is hard-coded here.
# Output is plain key=value lines (one record per line) so it can be fed
# straight into an AskUserQuestion option list.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DOSSIER="crates/starbreaker-ui/data/ui_screen_dossier_v1.json"
REF_ROOT="$HOME/projects/scorg_tools/reference/in-game"

case "${1:-}" in
    ""|-h|--help) sed -n '2,11p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
esac
SHIP_ARG="$1"
SCREEN="${2:-}"

# Resolve the ship folder case-insensitively against the dossier's own values.
SHIP=$(jq -r --arg s "$SHIP_ARG" \
    '[.screens[].ship_folder] | unique[] | select(ascii_downcase == ($s | ascii_downcase))' \
    "$DOSSIER" | head -1)
if [[ -z "$SHIP" ]]; then
    echo "error: no ship folder '$SHIP_ARG' in $DOSSIER" >&2
    jq -r '"known_ship=" + ([.screens[].ship_folder] | unique[])' "$DOSSIER" >&2
    exit 2
fi

if [[ -z "$SCREEN" ]]; then
    echo "ship=$SHIP"
    jq -r --arg ship "$SHIP" '
      .screens[] | select(.ship_folder == $ship) |
      "screen=\(.screen_id) tier=\(.tier // "-") lod=\(.lod // "-") " +
      "canvas=\(.canvas // "-") preset=\(.preset // "-") target=\(.target_id // "-")"
    ' "$DOSSIER"
    exit 0
fi

ROW=$(jq -r --arg ship "$SHIP" --arg id "$SCREEN" '
  .screens[] | select(.ship_folder == $ship and .screen_id == $id) |
  "screen_id=\(.screen_id)",
  "ship=\(.ship_folder)",
  "scene_package=\(.scene_package // "-")",
  "lod=\(.lod // "-")",
  "canvas=\(.canvas // "-")",
  "preset=\(.preset // "-")",
  "helper=\(.helper // "-")",
  "tier=\(.tier // "-")",
  "target_id=\(.target_id // "-")",
  "reference_file=\(.reference_file // "-")",
  "open_issues=\(.open_issues // "-")"
' "$DOSSIER")
if [[ -z "$ROW" ]]; then
    echo "error: screen '$SCREEN' not found for ship '$SHIP' in $DOSSIER" >&2
    exit 2
fi
echo "$ROW"

# Reference captures: every PNG whose name starts with the dossier reference
# stem (the straight-on/dark/rectified variants live beside the default one).
REF_FILE=$(sed -n 's/^reference_file=//p' <<<"$ROW")
STEM="${REF_FILE%.png}"
[[ "$STEM" == "-" ]] && STEM="$SCREEN"
found=0
for png in "$REF_ROOT/$SHIP/$STEM"*.png; do
    [[ -f "$png" ]] || continue
    corners=no
    [[ -f "$png.corners.json" ]] && corners=yes
    echo "reference=$png corners=$corners default=$([[ "$(basename "$png")" == "$REF_FILE" ]] && echo yes || echo no)"
    found=1
done
[[ "$found" -eq 1 ]] || echo "reference=none searched=$REF_ROOT/$SHIP/$STEM*.png"
