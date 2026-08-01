#!/usr/bin/env bash
# perf_bisect.sh — benchmark ONE historical commit, then put HEAD back.
#
# RESPONSIBILITY
#   "The docs say it was Xs" is not a baseline (rule 3): to compare two points in
#   history you must BUILD both binaries and time them back-to-back on this
#   machine. This detaches to <sha>, builds release, runs the standard
#   perf_bench.sh ritual under a <label>-<shortsha> tag, prints the built binary's
#   provenance (rule 7 — an identical sha256 across endpoints means you timed the
#   SAME binary twice), and ALWAYS restores the original HEAD via an EXIT trap.
#   Refuses to run on a dirty tree: a detach would strand your work.
#
#   FREE no-rebuild ladder first: old hashed executables from previous builds live
#   under `target/release/deps/starbreaker-<hash>` — time those directly with
#   perf_bench.sh's outdir/logs (they time-travel-bisect without any rebuild, and
#   perf_provenance.sh on each tells you which is which). Only reach for this
#   script when the point in history you want has no surviving hashed binary.
#
# USAGE
#   perf_bisect.sh <sha> <entity> <label>
#     <sha>     commit to benchmark (anything git rev-parse accepts)
#     <entity>  entity name, e.g. anvl_carrack (never defaulted)
#     <label>   endpoint tag; the bench runs as <label>-<shortsha>
#
#   Machine must be QUIET. SC_DATA_P4K is NOT exported — the P4K is auto-detected.
set -euo pipefail

_verdict=""
_orig_ref=""
_restore_and_marker() {
  local rc=$?
  if [[ -n "$_orig_ref" ]]; then
    echo
    echo "=== restoring original HEAD: $_orig_ref ==="
    git -C "$repo_root" checkout --force "$_orig_ref" >&2 || {
      echo "BISECT: FAILED (could not restore $_orig_ref — repo left detached, fix by hand)"; exit 1; }
  fi
  [[ -n "$_verdict" ]] && echo "$_verdict" || echo "BISECT: FAILED (rc=$rc)"
  exit "$rc"
}

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"

usage() { sed -n '2,25p' "$0" | sed 's/^# \{0,1\}//'; }

case "${1:-}" in
  "" | -h | --help) usage; exit 0 ;;
esac

trap _restore_and_marker EXIT

[[ $# -eq 3 ]] || { echo "perf_bisect: need <sha> <entity> <label> (try --help)" >&2; exit 64; }
sha="$1"; entity="$2"; label="$3"

cd "$repo_root"

# --- refuse on a dirty tree -------------------------------------------------
if [[ -n "$(git status --porcelain)" ]]; then
  echo "perf_bisect: REFUSING — working tree is dirty; commit or stash first (a detach would strand it):" >&2
  git status --short >&2
  exit 1
fi

# Symbolic ref if on a branch, else the detached sha — this is what we restore.
_orig_ref="$(git symbolic-ref --quiet --short HEAD || git rev-parse HEAD)"
echo "=== original HEAD: $_orig_ref ($(git rev-parse --short HEAD)) ==="

target_sha="$(git rev-parse --verify "$sha^{commit}")"
short="$(git rev-parse --short "$target_sha")"

echo "=== checking out $short (detached) ==="
git checkout --detach "$target_sha"
git --no-pager log -1 --oneline

echo
echo "=== cargo build --release -p starbreaker ==="
cargo build --release -p starbreaker

echo
echo "=== provenance of the binary just built at $short ==="
"$script_dir/perf_provenance.sh" "$repo_root/target/release/starbreaker"

echo
echo "=== bench ==="
"$script_dir/perf_bench.sh" "$entity" "${label}-${short}"

_verdict="BISECT: OK ($short, label ${label}-${short})"
