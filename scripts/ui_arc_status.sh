#!/usr/bin/env bash
# One command per UI-parity loop cycle (plan A3): render one screen, compare it
# to its in-game reference over the dossier's region preset, and flag which
# regions CHANGED / are NEW / are the same vs the previous cycle — so an agent
# vision-reads ONLY the flagged crops instead of the whole screen every time.
#
# Usage:
#   bash scripts/ui_arc_status.sh --screen <screen_id> [--no-render]
#
#   --no-render   reuse the latest render (skip the ~1-min build+render); use it
#                 for the immediate re-check after a render to confirm no drift.
#
# State: /tmp/ui_arc_status/<screen_id>/{cur.json,prev.json}. The dossier
# (crates/starbreaker-ui/data/ui_screen_dossier_v1.json) supplies the reference,
# preset and target for the screen. A screen with no preset fails loudly.
set -euo pipefail
# RESULT MARKER on failure (mirrors ui_check.sh): a piped exit code is the
# filter's, not ours, so emit a distinct FAILED line on any non-zero exit. The
# OK line is printed by the body (rc still 0), so the trap only handles failure.
trap 'rc=$?; if [[ $rc -ne 0 ]]; then echo; echo "ui_arc_status: FAILED (exit $rc) — see output above"; fi' EXIT

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$REPO_ROOT"

DOSSIER="crates/starbreaker-ui/data/ui_screen_dossier_v1.json"
BANK="crates/starbreaker-ui/tests/fixtures/ui_ir/reference_measurements_v1.json"
REF_ROOT="$HOME/projects/scorg_tools/reference/in-game"

SCREEN_ID=""
NO_RENDER=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --screen) SCREEN_ID="$2"; shift 2 ;;
        --no-render) NO_RENDER=1; shift ;;
        -h|--help) sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown argument: $1" >&2; exit 2 ;;
    esac
done
[[ -n "$SCREEN_ID" ]] || { echo "error: --screen <screen_id> is required (see $DOSSIER)" >&2; exit 2; }

# Dossier row -> ship_folder, reference_file, preset, helper, target_id (one per
# line; null/empty -> empty line). Same lookup shape as ui_render.sh (A2).
LOOKUP=$(python3 - "$DOSSIER" "$SCREEN_ID" <<'PY'
import json, sys
d = json.load(open(sys.argv[1])); sid = sys.argv[2]
rows = [s for s in d["screens"] if s["screen_id"] == sid]
if not rows:
    sys.exit(3)
s = rows[0]
for k in ("ship_folder", "reference_file", "preset", "helper", "target_id"):
    v = s.get(k)
    print(v if v not in (None, "") else "")
print("__END__")   # sentinel: keeps trailing empty fields (preset/target_id null)
                   # from being stripped by $(...) so the last read never EOFs
PY
) || { echo "error: screen '$SCREEN_ID' not in $DOSSIER — add its row (and ui-reference §3)" >&2; exit 2; }
{ IFS= read -r SHIP; IFS= read -r REF_FILE; IFS= read -r PRESET; IFS= read -r HELPER; IFS= read -r TARGET_ID; } <<<"$LOOKUP"

[[ -n "$PRESET" ]] || { echo "error: screen '$SCREEN_ID' has no ui_compare preset — add a preset for this screen first (ui_compare presets: python3 scripts/ui_compare.py --regions list)" >&2; exit 2; }
[[ -n "$REF_FILE" ]] || { echo "error: screen '$SCREEN_ID' has no reference capture in the dossier (mirror / no straight-on capture) — compare its sibling instead" >&2; exit 2; }

# Reference: dossier default, unless a straight-on variant carrying a
# .corners.json sidecar exists (ui-reference §3: rectifiable captures win).
REF="$REF_ROOT/$SHIP/$REF_FILE"
if [[ ! -f "$REF.corners.json" ]]; then
    stem="${REF_FILE%.png}"
    for cand in "$REF_ROOT/$SHIP/$stem"*.png; do
        [[ -f "$cand.corners.json" ]] && { REF="$cand"; break; }
    done
fi
[[ -f "$REF" ]] || { echo "error: reference not found: $REF" >&2; exit 2; }

# Export-stamp age (info only — the visual guard owns the authoritative check).
STAMP_FILE="$HOME/projects/scorg_tools/ships/Data/UI/Generated/.export_stamp.json"
if [[ -f "$STAMP_FILE" ]]; then
    AGE_MIN=$(( ( $(date +%s) - $(python3 -c "import json;print(json.load(open('$STAMP_FILE'))['written_at_epoch_s'])") ) / 60 ))
    echo "export stamp age: ${AGE_MIN}min (info)"
fi

# Render (unless --no-render, which reuses the latest).
if [[ "$NO_RENDER" -eq 0 ]]; then
    bash scripts/ui_render.sh --screen "$SCREEN_ID"
fi
RENDER_DIR="/tmp/ui_render/$HELPER/latest"
RENDER_PNG=$(ls "$RENDER_DIR"/*.png 2>/dev/null | head -1 || true)
[[ -n "$RENDER_PNG" && -f "$RENDER_PNG" ]] || {
    echo "error: no render at $RENDER_DIR — drop --no-render (nothing rendered yet for '$HELPER')" >&2
    exit 2
}

STATE_DIR="/tmp/ui_arc_status/$SCREEN_ID"
mkdir -p "$STATE_DIR/cmp"
CUR="$STATE_DIR/cur.json"
PREV="$STATE_DIR/prev.json"

# Compare to a fresh file first, THEN rotate — a failed compare leaves cur/prev
# intact.
NEWCUR="$STATE_DIR/cur.new.json"
python3 scripts/ui_compare.py "$RENDER_PNG" "$REF" \
    --regions "$PRESET" --stats --json "$NEWCUR" --out-dir "$STATE_DIR/cmp" >/dev/null
[[ -f "$CUR" ]] && mv "$CUR" "$PREV"
mv "$NEWCUR" "$CUR"

echo
echo "==> $SCREEN_ID  (preset $PRESET${TARGET_ID:+, target $TARGET_ID})  ref: $REF"
PREV_ARGS=()
[[ -f "$PREV" ]] && PREV_ARGS=(--prev "$PREV")
SUMMARY=$(python3 scripts/ui_region_summary.py --cur "$CUR" "${PREV_ARGS[@]}" \
    --bank "$BANK" --reference "$REF_FILE")
echo "$SUMMARY"

NREG=0; MCHG=0
if [[ "$SUMMARY" =~ OK\ \(([0-9]+)\ regions,\ ([0-9]+)\ changed\) ]]; then
    NREG="${BASH_REMATCH[1]}"; MCHG="${BASH_REMATCH[2]}"
fi
echo
echo "ui_arc_status: OK ($NREG regions, $MCHG changed)"
