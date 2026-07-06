#!/usr/bin/env bash
# Build-then-replay wrapper (crates/starbreaker-ui/docs/ui-process-improvements.md Part C).
#
# Twice in one session a render silently used a STALE binary because the
# preceding `cargo build` ran in the wrong working directory (background
# shells reset their cwd) — the renders looked identical and nearly
# mis-adjudicated a fix as ineffective. This wrapper resolves the repo root
# from its own location, always builds first, and prints the binary mtime so
# staleness is visible.
#
# Usage:
#   bash scripts/ui_render.sh --helper <name> [--screen <screen_id>] \
#       [--lod 0|1] [--scene <scene.json>] [--out <dir>] [--ir]
#
# Scene: --scene wins; else the screen dossier
#   (crates/starbreaker-ui/data/ui_screen_dossier_v1.json) resolves the helper
#   (or --screen <screen_id>) to its scene_package + lod. An unknown helper
#   with no dossier row and no --scene is a hard error naming the dossier —
#   this is what stops a non-Clipper ship (e.g. Carrack) silently rendering a
#   Clipper scene, which the old hard-coded LOD0/LOD1 constants + case-list did.
# Output: default /tmp/ui_render/<helper>/<UTC-stamp>/ (each run fresh, never
#   clobbered); /tmp/ui_render/<helper>/latest symlinks the most recent run.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DOSSIER="crates/starbreaker-ui/data/ui_screen_dossier_v1.json"
SCENE=""        # --scene overrides the dossier lookup
LOD=""          # --lod 0|1; else the dossier row's lod
HELPER=""
SCREEN_ID=""    # --screen <screen_id>: alternative dossier key
OUT=""
OUT_EXPLICIT=0
DUMP_IR=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --helper) HELPER="$2"; shift 2 ;;
        --screen) SCREEN_ID="$2"; shift 2 ;;
        --scene) SCENE="$2"; shift 2 ;;
        --lod) LOD="$2"; shift 2 ;;
        --out) OUT="$2"; OUT_EXPLICIT=1; shift 2 ;;
        --ir) DUMP_IR=1; shift ;;
        -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done
if [[ -z "$HELPER" && -z "$SCREEN_ID" ]]; then
    echo "error: --helper (or --screen <screen_id>) is required (see the screen dossier: $DOSSIER)" >&2
    exit 2
fi

# Scene: explicit --scene wins; else the dossier resolves helper/screen ->
# scene_package + lod. scene_package names contain a space ("DRAK Clipper_..."),
# so read the fields line-by-line, NOT `read PKG LOD` (which would word-split it).
if [[ -z "$SCENE" ]]; then
    LOOKUP=$(python3 - "$DOSSIER" "$HELPER" "$SCREEN_ID" <<'PY'
import json, sys
d = json.load(open(sys.argv[1])); h = sys.argv[2]; sid = sys.argv[3]
rows = [s for s in d["screens"] if (h and s["helper"] == h) or (sid and s["screen_id"] == sid)]
if not rows:
    sys.exit(3)
s = rows[0]
print(s["scene_package"])   # line 1 (contains spaces)
print(s["lod"])             # line 2
print(s["helper"])          # line 3 (names the output dir when only --screen given)
PY
    ) || { echo "error: helper '${HELPER}'${SCREEN_ID:+ / screen '${SCREEN_ID}'} not in $DOSSIER — add its row (and ui-reference §3) or pass --scene" >&2; exit 2; }
    { IFS= read -r PKG; IFS= read -r DLOD; IFS= read -r DHELPER; } <<<"$LOOKUP"
    [[ -n "$LOD" ]] || LOD="$DLOD"
    [[ -n "$HELPER" ]] || HELPER="$DHELPER"
    SCENE="$HOME/projects/scorg_tools/ships/Packages/$PKG/scene.json"
fi
[[ -n "$HELPER" ]] || HELPER="$SCREEN_ID"   # --scene + --screen, no --helper
echo "==> scene (LOD${LOD:-?}): $SCENE"

echo "==> cargo build (debug)"
cargo build
echo "==> binary: $(stat -c '%y' target/debug/starbreaker | cut -d. -f1)"

# Unique output dir per run (ledger 69 extended): distinct timestamped dirs stop
# a viewer/tool cache-colliding on a fixed path, and keep prior renders for diff.
STAMP=$(date -u +%Y%m%dT%H%M%SZ)
[[ -n "$OUT" ]] || OUT="/tmp/ui_render/$HELPER/$STAMP"
[[ "$OUT_EXPLICIT" -eq 1 ]] && rm -rf "$OUT"   # explicit --out: caller owns it, start fresh
mkdir -p "$OUT"

ARGS=(ui render --scene "$SCENE" --out-dir "$OUT" --helper "$HELPER")
if [[ "$DUMP_IR" -eq 1 ]]; then
    ARGS+=(--dump-ir-dir "$OUT/ir")
fi
./target/debug/starbreaker "${ARGS[@]}"

mkdir -p "/tmp/ui_render/$HELPER"
ln -sfn "$OUT" "/tmp/ui_render/$HELPER/latest"
ls -1 "$OUT"
# Print the rendered PNG's md5 (ledger 69): an "unchanged-looking" render can be
# confirmed as actually-changed-on-disk — distinguishing a real no-op (same hash)
# from a viewer-cache artifact (different hash, same-looking image). Each run's
# dir is already unique, so the viewer can't cache-collide on the output path.
for png in "$OUT"/*.png; do
    [[ -f "$png" ]] && printf 'png md5: %s  %s\n' "$(md5sum "$png" | cut -d' ' -f1)" "$png"
done
echo "latest -> $OUT"
