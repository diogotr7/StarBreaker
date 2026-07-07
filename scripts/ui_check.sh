#!/usr/bin/env bash
# Standard starbreaker-ui check battery (crates/starbreaker-ui/docs/ui-process-improvements.md item 4).
#
# Default (TDD tier) — run after every red/green cycle:
#   example compile check + the WHOLE starbreaker-ui test suite (lib + every
#   integration test target: guards, swf_*, pipeline_ir, brand palettes, …).
#   The two export-coupled visual guards (whole-image colour + custom-shape)
#   are skipped here via UI_SKIP_VISUAL_GUARD=1 — they need a fresh export and
#   run only in --full. Running the whole suite is what catches compile errors
#   and logic regressions in targets the old hand-picked list silently skipped
#   (e.g. swf_phase5_wiring + the pipeline_ir slot-9 regression, 2026-06-13).
#
# --full (workstream-boundary tier) — adds:
#   the export-coupled visual guards run AUTHORITATIVELY (no skip env),
#   freeze + artifact validators, starbreaker-3d lib tests, and the
#   font-size harness (crates/starbreaker-ui/docs/ui-font-size-harness.md) against UI_CHECK_SCENE.
#
# Environment:
#   UI_CHECK_SCENE   scene.json used for the --full font harness replay.
#                    Default: the Clipper LOD1 interior scene (it carries the
#                    medical/door/annunciator bindings the font baseline
#                    covers; the LOD0 cockpit scene does NOT).
set -euo pipefail
# Emit a DISTINCT final marker on every exit AND persist it to a marker file the
# pre-commit UI gate reads. Two problems it solves:
#   1. Piping the run through `| tail`/`| grep` (common) reports the FILTER's
#      exit code (0), so a real failure looks green and a background-task
#      notification says "exit code 0" on a failing run (ledger 89). The console
#      marker survives the pipe, so the pass/fail signal is unambiguous.
#   2. The marker FILE ($git-dir/ui-check-marker) lets the pre-commit gate
#      (scripts/hooks/pre-commit-ui-gate) refuse to commit renderer/script/mcp
#      changes unless the LAST ui_check was genuinely green and recent — a
#      remembered exit code can never be trusted again.
# The console output is unchanged: this trap still prints the FAILED line on
# failure, and "ui_check: ALL GREEN" stays a plain echo at the end on success.
# A marker-file write error must NEVER flip the script's own exit status.
_ui_check_marker() {
  local rc=$?
  local status
  if [[ $rc -eq 0 ]]; then
    status="ui_check: ALL GREEN"
  else
    status="ui_check: FAILED (exit $rc)"
    echo
    echo "$status — see output above"
  fi
  local git_dir head
  if git_dir="$(git rev-parse --git-dir 2>/dev/null)"; then
    head="$(git rev-parse HEAD 2>/dev/null || echo unknown)"
    printf '%s\nhead=%s\nepoch=%s\n' "$status" "$head" "$(date +%s)" \
      > "$git_dir/ui-check-marker" 2>/dev/null || true
  fi
  exit "$rc"
}
trap _ui_check_marker EXIT
cd "$(dirname "$0")/.."

FULL=0
for arg in "$@"; do
  case "$arg" in
    --full) FULL=1 ;;
    -h|--help)
      sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *) echo "unknown argument: $arg (try --help)" >&2; exit 64 ;;
  esac
done

step() { echo; echo "==> $*"; }

# Examples are diagnostics tooling; a broken example otherwise stays unnoticed
# until someone reaches for it mid-investigation (ledger item 26).
step "starbreaker-ui examples compile"
cargo check -p starbreaker-ui --examples

# Repo-only (no game data): the screen dossier must not drift from ui-reference
# §3 (registry pattern). Fast; runs in both tiers.
step "validate_ui_dossier (screen dossier <-> ui-reference §3)"
python3 scripts/validate_ui_dossier.py

if [[ "$FULL" == 1 ]]; then
  # Early staleness visibility (ledger item 30): the visual guard hard-fails
  # when the test binary is >30min newer than the export stamp — surface the
  # stamp age BEFORE minutes of suites run, so a needed re-export (~50s)
  # happens first. Warning only; the in-guard check stays authoritative.
  STAMP="$HOME/projects/scorg_tools/ships/Data/UI/Generated/.export_stamp.json"
  if [[ -f "$STAMP" ]]; then
    AGE_MIN=$(( ( $(date +%s) - $(python3 -c "import json;print(json.load(open('$STAMP'))['written_at_epoch_s'])") ) / 60 ))
    echo "export stamp age: ${AGE_MIN}min"
    if (( AGE_MIN > 30 )); then
      echo "WARNING: export stamp is ${AGE_MIN}min old — if the ui test binaries rebuild now, the staleness guard WILL fail; re-export first (~50s)." >&2
    fi
  else
    echo "WARNING: no export stamp at $STAMP — the visual guard will fail unless game data is absent (skip path)." >&2
  fi

  step "starbreaker-ui FULL test suite (export-coupled visual guards authoritative)"
  cargo test -p starbreaker-ui

  step "validate_ui_snapshot_freeze"
  bash scripts/validate_ui_snapshot_freeze.sh

  step "validate_ui_regression_artifacts --quick"
  bash scripts/validate_ui_regression_artifacts.sh --quick

  step "starbreaker-3d lib tests"
  cargo test -p starbreaker-3d --lib

  SCENE="${UI_CHECK_SCENE:-$HOME/projects/scorg_tools/ships/Packages/DRAK Clipper_LOD1_TEX2/scene.json}"
  if [[ -f "$SCENE" ]]; then
    step "font-size harness (scene: $SCENE)"
    DUMP="$(mktemp /tmp/ui_check_fontdump.XXXXXX.tsv)"
    SB_UI_FONT_DUMP=1 cargo run -q -p starbreaker -- ui render --scene "$SCENE" \
      --out-dir "$(mktemp -d /tmp/ui_check_render.XXXXXX)" 2>&1 \
      | grep '^FONTDUMP' > "$DUMP"
    python3 scripts/font_size_check.py "$DUMP"
  else
    echo "SKIP font harness: scene not found: $SCENE (set UI_CHECK_SCENE)" >&2
  fi
else
  # TDD tier: the WHOLE crate suite (lib + every integration target), with the
  # two export-coupled visual guards skipped — they need a fresh export and are
  # exercised authoritatively in --full. Running the whole suite (vs the old
  # hand-picked --test list) is what surfaces breakage in every target.
  step "starbreaker-ui test suite (export-coupled visual guards skipped — see --full)"
  UI_SKIP_VISUAL_GUARD=1 cargo test -p starbreaker-ui
fi

echo
echo "ui_check: ALL GREEN"
