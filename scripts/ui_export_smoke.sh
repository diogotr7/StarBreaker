#!/usr/bin/env bash
# ui_export_smoke.sh — composed-export smoke gate (alignment plan T7).
#
# RESPONSIBILITY
#   Verify the crate's ACTUAL deliverable — the baked screen PNGs of the
#   decomposed export — end-to-end through the PRODUCTION UI fetchers, not the
#   unit mocks. Guards this failure class: unit guards stayed green while every
#   MFD exported BLANK because a mock interface production never overrode the
#   real one. This gate reads only real export output.
#
# WHAT IT READS (exact paths / fields — Step-1 research)
#   Screen PNGs : <export_root>/Data/UI/Generated/**/*.png
#                 (written by generated_ui_binding_path(), decomposed.rs:3345:
#                  Data/UI/Generated/{ship|vehicle}/{mfr}/{ship}/{asset}.png)
#   Enumerator  : <export_root>/Packages/*/scene.json — every UI binding record's
#                 "generated_image_path" field (serialized decomposed.rs:2429;
#                 set in generated_ui_binding_record() ONLY on a successful
#                 production render). The DISTINCT non-null set is the
#                 UiRenderKey-deduped list of render targets and maps 1:1 to the
#                 PNG files on disk. The expected count is DERIVED from this set
#                 at run time — never a hard-coded number.
#
# ASSERTIONS
#   (a) NO binding record has generated_image_path: null — the field is set only
#       in the render Ok arm (decomposed.rs:3190), so a null IS the Err arm: a
#       screen that failed to bake. The enumeration alone is success-conditioned
#       and would silently shrink; this catches the Err class structurally.
#   (b) N := |distinct generated_image_path| > 0, and every one of those N paths
#       exists as a PNG on disk (presence: a render target with no baked file).
#   (c) every one of those N PNGs has > 1 distinct pixel value (a uniform image
#       is a dead / blank render — the blank-MFD signature).
#   WARN (not fail): on-disk PNGs under the enumerated Generated subtrees that
#       appear in NO gated scene.json — stale files across re-exports are
#       legitimate, but were previously invisible (e.g. a stale blank provider
#       PNG the success-conditioned enumeration never saw).
#   Marker: `ui_export_smoke: OK (N screens)` / `ui_export_smoke: FAILED`.
#
# MODES
#   ui_export_smoke.sh <entity>          UI-only export of <entity> to a temp dir
#                                        (invocation lifted from
#                                        benchmark_ui_only_export.sh), then check.
#   ui_export_smoke.sh --export-root DIR [--packages GLOB]
#                                        check a pre-existing export root, NO
#                                        export (used by ui_check.sh --full, which
#                                        reads ships/Data/UI/Generated guarded by
#                                        the export stamp — never re-exports).
#                                        GLOB (default '*') scopes Packages/<glob>
#                                        so the check tracks the entity under test.
#   ui_export_smoke.sh --self-test       blank-detector unit check (no game data).
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"

# Non-empty precondition helper (T5): the DERIVED expected set must assert N > 0,
# or a rename/decompose that yields zero targets would pass vacuously.
source "$script_dir/lib/guard_assert.sh"

# RESULT MARKER via EXIT trap — survives `| tail`/`| grep` (ledger 89): the pipe
# would otherwise report the filter's exit code and hide a real failure.
SMOKE_SUMMARY=""
TMP_ROOT=""   # set in entity mode; removed by the marker on any exit.
_ui_export_smoke_marker() {
  local rc=$?
  [[ -n "$TMP_ROOT" ]] && rm -rf "$TMP_ROOT"
  if [[ $rc -eq 0 ]]; then
    echo "ui_export_smoke: OK (${SMOKE_SUMMARY:-done})"
  else
    echo "ui_export_smoke: FAILED"
  fi
  exit "$rc"
}
trap _ui_export_smoke_marker EXIT

# --- blank detector ---------------------------------------------------------
# Exit 0 if the PNG has > 1 distinct pixel value (a real render), non-zero if it
# is uniform (dead / blank). PIL getcolors(2) -> None when > 2 colours (clearly
# non-blank), else a list whose length is the exact colour count.
png_is_nonblank() {
  python3 - "$1" <<'PY'
import sys
from PIL import Image
cols = Image.open(sys.argv[1]).getcolors(2)   # None if > 2 colours
sys.exit(0 if (cols is None or len(cols) > 1) else 1)
PY
}

# --- self-test --------------------------------------------------------------
self_test() {
  local dir; dir="$(mktemp -d)"
  trap 'rm -rf "$dir"' RETURN
  python3 - "$dir" <<'PY'
import sys
from PIL import Image
d = sys.argv[1]
Image.new("RGB", (4, 4), (10, 20, 30)).save(f"{d}/uniform.png")      # 1 colour
im = Image.new("RGB", (4, 4), (10, 20, 30)); im.putpixel((0, 0), (200, 100, 50))
im.save(f"{d}/twocolour.png")                                        # 2 colours
PY
  echo "ui_export_smoke self-test:"
  if png_is_nonblank "$dir/twocolour.png"; then
    echo "  ok: 2-colour PNG detected as non-blank"
  else
    echo "  FAIL: 2-colour PNG wrongly flagged blank" >&2; exit 2
  fi
  if png_is_nonblank "$dir/uniform.png"; then
    echo "  FAIL: uniform PNG wrongly passed as non-blank" >&2; exit 2
  else
    echo "  ok: uniform PNG detected as blank"
  fi

  # Null-path failure: a synthetic export root whose scene.json carries one
  # binding with generated_image_path null (render Err arm) alongside one valid
  # baked binding — run_check must FAIL and name the null, not pass on the 1
  # good screen.
  local synth="$dir/root"
  mkdir -p "$synth/Data/UI/Generated/synth" "$synth/Packages/synth_pkg"
  cp "$dir/twocolour.png" "$synth/Data/UI/Generated/synth/ok.png"
  cat > "$synth/Packages/synth_pkg/scene.json" <<'JSON'
{"placements":[{"ui_bindings":[
  {"binding_kind":"mfd","generated_image_path":null},
  {"binding_kind":"mfd","generated_image_path":"Data/UI/Generated/synth/ok.png"}
]}]}
JSON
  local out rc=0
  out="$( (run_check "$synth") 2>&1 )" || rc=$?
  if [[ $rc -ne 0 && "$out" == *"null generated_image_path"* ]]; then
    echo "  ok: null generated_image_path fails loudly"
  else
    echo "  FAIL: null-path scene not rejected (rc=$rc out='$out')" >&2; exit 2
  fi
  SMOKE_SUMMARY="self-test"
}

# --- core check over an export root -----------------------------------------
# run_check <export_root> [packages_glob]
#   packages_glob (default '*') selects which Packages/<glob>/scene.json to
#   enumerate — the check is scoped to the entity/ship under test, matching the
#   brief's per-entity model. ui_check.sh --full passes the Clipper glob so a
#   stale sibling-ship export cannot red-line the tier.
run_check() {
  local root="$1" pkg_glob="${2:-*}"
  [[ -d "$root/Data/UI/Generated" ]] || { echo "no Data/UI/Generated under $root" >&2; exit 1; }

  # Derive the expected render-target set from scene.json (see header), plus:
  # NULLS — binding records whose generated_image_path is null (render Err arm);
  # ORPHAN — on-disk PNGs under the enumerated subtrees in no gated scene.json.
  mapfile -t enum_lines < <(
    python3 - "$root" "$pkg_glob" <<'PY'
import json, sys, os, glob
root, pkg_glob = sys.argv[1], sys.argv[2]
paths, nulls = set(), 0
def walk(o):
    global nulls
    if isinstance(o, dict):
        if "generated_image_path" in o:          # binding record shape
            p = o["generated_image_path"]
            if p:
                paths.add(p)
            else:
                nulls += 1
        for v in o.values():
            walk(v)
    elif isinstance(o, list):
        for v in o:
            walk(v)
for s in glob.glob(os.path.join(root, "Packages", pkg_glob, "scene.json")):
    walk(json.load(open(s)))
print("NULLS", nulls)
disk = set()
for d in {os.path.dirname(p) for p in paths}:    # only the gated subtrees
    for dp, _, fns in os.walk(os.path.join(root, d)):
        for fn in fns:
            if fn.lower().endswith(".png"):
                disk.add(os.path.relpath(os.path.join(dp, fn), root))
for o in sorted(disk - paths):
    print("ORPHAN", o)
for p in sorted(paths):
    print("TARGET", p)
PY
  )

  local null_count=0 expected=() orphans=() line
  for line in "${enum_lines[@]}"; do
    case "$line" in
      "NULLS "*)  null_count="${line#NULLS }" ;;
      "ORPHAN "*) orphans+=("${line#ORPHAN }") ;;
      "TARGET "*) expected+=("${line#TARGET }") ;;
    esac
  done

  if (( null_count > 0 )); then
    echo "FAIL: $null_count binding record(s) with null generated_image_path — render Err arm, screen(s) never baked" >&2
    exit 1
  fi

  local n=${#expected[@]}
  assert_nonzero_matches "$n" "UI render targets (generated_image_path) in $root scene.json"

  if (( ${#orphans[@]} > 0 )); then
    echo "WARNING: ${#orphans[@]} on-disk PNG(s) under the gated Generated subtree(s) in no scene.json (stale from a prior export?):" >&2
    printf '  ORPHAN: %s\n' "${orphans[@]}" >&2
  fi

  local missing=0 blank=0 p
  for p in "${expected[@]}"; do
    local f="$root/$p"
    if [[ ! -f "$f" ]]; then
      echo "MISSING baked PNG for render target: $p" >&2; missing=$((missing + 1)); continue
    fi
    if ! png_is_nonblank "$f"; then
      echo "BLANK (uniform) baked PNG: $p" >&2; blank=$((blank + 1))
    fi
  done

  if (( missing > 0 || blank > 0 )); then
    echo "smoke check failed: $missing missing, $blank blank of $n render targets" >&2
    exit 1
  fi
  SMOKE_SUMMARY="$n screens"
}

# --- entrypoint -------------------------------------------------------------
case "${1:-}" in
  --self-test)
    self_test
    ;;
  --export-root)
    # --export-root DIR [--packages GLOB]  (check pre-existing export, no export)
    [[ -n "${2:-}" ]] || { echo "usage: $0 --export-root <dir> [--packages <glob>]" >&2; exit 64; }
    root="$2"; pkg_glob="*"
    if [[ "${3:-}" == "--packages" ]]; then
      [[ -n "${4:-}" ]] || { echo "usage: $0 --export-root <dir> [--packages <glob>]" >&2; exit 64; }
      pkg_glob="$4"
    fi
    run_check "$root" "$pkg_glob"
    ;;
  "" | -h | --help)
    sed -n '2,49p' "$0" | sed 's/^# \{0,1\}//'
    trap - EXIT   # --help runs no checks: don't mint a false OK marker.
    exit 0
    ;;
  --*)
    echo "unknown option: $1 (try --help)" >&2; exit 64
    ;;
  *)
    entity="$1"
    TMP_ROOT="$(mktemp -d "/tmp/ui_export_smoke_${entity}.XXXXXX")"  # marker cleans up
    # UI-only export (invocation lifted from benchmark_ui_only_export.sh). Game
    # data auto-detects; SC_DATA_P4K is honoured if the env sets it.
    pushd "$repo_root" >/dev/null
    cargo run --release -p starbreaker -- entity export "$entity" "$TMP_ROOT" \
      --kind decomposed --lod 0 --mip 0 --materials all --ui-only-files
    popd >/dev/null
    run_check "$TMP_ROOT"
    ;;
esac
