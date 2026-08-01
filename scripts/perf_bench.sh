#!/usr/bin/env bash
# perf_bench.sh — the standard baseline/re-baseline benchmark ritual.
#
# RESPONSIBILITY
#   Run the release `entity export` N times under `/usr/bin/time -v`, extract the
#   only numbers that count (wall, CPU%, max RSS, the `[timing]` stage breakdown)
#   and print a per-run + MEDIAN table. This ritual was hand-typed dozens of times
#   across optimisation passes; encoding it removes the two ways it went wrong:
#   forgetting the binary's PROVENANCE (measurement rule 7 — a stale binary
#   masquerading as a fresh baseline cost whole sessions), and writing multi-GB
#   export dirs into /tmp (tmpfs) until a later run died mid-write and poisoned
#   the comparison (measurement rule 10). Outputs default under the workspace
#   `target/tmp`, never /tmp.
#
# USAGE
#   perf_bench.sh <entity> <label> [--runs 3] [--outdir DIR]
#     <entity>    entity name, e.g. anvl_carrack (never defaulted — always yours)
#     <label>     tag for this endpoint, e.g. base / post-item14
#     --runs N    number of runs (default 3; the skill requires 3+)
#     --outdir D  where bench dirs+logs go (default <repo>/target/tmp)
#
#   Machine must be QUIET (the skill GATES benchmarking on asking the user).
#   Timing runs carry NO --mem-cap (rule 4: it adds allocation-tracking overhead).
#   SC_DATA_P4K is NOT exported — the P4K is auto-detected.
set -euo pipefail

_marker() {
  local rc=$?
  [[ $rc -eq 0 ]] && echo "BENCH: OK" || echo "BENCH: FAILED (rc=$rc)"
  exit "$rc"
}
trap _marker EXIT

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$script_dir/.." && pwd)"

usage() { sed -n '2,24p' "$0" | sed 's/^# \{0,1\}//'; }

case "${1:-}" in
  "" | -h | --help) usage; trap - EXIT; exit 0 ;;
esac

entity="$1"; shift
[[ $# -ge 1 && "$1" != --* ]] || { echo "perf_bench: <label> is required (try --help)" >&2; exit 64; }
label="$1"; shift

runs=3
outdir="$repo_root/target/tmp"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --runs)   runs="${2:?--runs needs a value}"; shift 2 ;;
    --outdir) outdir="${2:?--outdir needs a value}"; shift 2 ;;
    *) echo "unknown option: $1 (try --help)" >&2; exit 64 ;;
  esac
done
[[ "$runs" =~ ^[0-9]+$ && "$runs" -ge 1 ]] || { echo "perf_bench: --runs must be a positive integer" >&2; exit 64; }

bin="$repo_root/target/release/starbreaker"
[[ -x "$bin" ]] || { echo "perf_bench: no release binary at $bin (cargo build --release -p starbreaker)" >&2; exit 1; }

mkdir -p "$outdir"

# --- preflight: quiet machine ----------------------------------------------
echo "=== preflight ==="
uptime
cores="$(nproc)"
echo "nproc:  $cores"
load1="$(awk '{print $1}' /proc/loadavg)"
if awk -v l="$load1" -v c="$cores" 'BEGIN { exit !(l > c/4) }'; then
  echo "  WARNING: 1-min load $load1 > nproc/4 ($((cores / 4))) — machine is BUSY; timings will be noise (rule 3)" >&2
fi

# --- preflight: disk (rule 10 — a full fs poisons the comparison) ----------
avail_k="$(df -Pk "$outdir" | awk 'NR==2 {print $4}')"
avail_g=$((avail_k / 1024 / 1024))
echo "outdir: $outdir (${avail_g}G free on $(df -P "$outdir" | awk 'NR==2 {print $1" @ "$6}'))"
if [[ "$avail_g" -lt 10 ]]; then
  echo "perf_bench: REFUSING — only ${avail_g}G free on the outdir filesystem, need >= 10G (ledger: a run that dies mid-write poisons the comparison)" >&2
  exit 1
fi

# --- provenance (rule 7 — run on BOTH endpoints and diff the outputs) ------
echo
echo "=== provenance (this endpoint: $label) ==="
"$script_dir/perf_provenance.sh" "$bin"

# --- runs -------------------------------------------------------------------
walls=()
for ((r = 1; r <= runs; r++)); do
  dir="$outdir/bench_${label}_$r"
  log="$outdir/bench_${label}_$r.log"
  rm -rf "$dir"
  echo
  echo "=== run $r/$runs -> $dir ==="
  /usr/bin/time -v env RUST_LOG=info "$bin" \
    entity export "$entity" "$dir" --kind decomposed --lod 0 --mip 0 --materials all \
    > "$log" 2>&1 || { echo "perf_bench: run $r FAILED — see $log" >&2; tail -20 "$log" >&2; exit 1; }

  grep -aE "Elapsed \(wall|Percent of CPU|Maximum resident|\[timing\]" "$log" || true

  # h:mm:ss(.ss) or m:ss(.ss) -> seconds
  wall="$(grep -a "Elapsed (wall" "$log" | awk -F': ' '{print $NF}' \
    | awk -F: '{ s=0; for (i=1; i<=NF; i++) s = s*60 + $i; printf "%.2f", s }')"
  walls+=("$wall")
  echo "run $r wall: ${wall}s"
done

# --- summary ----------------------------------------------------------------
echo
echo "=== summary: $label ($entity, $runs runs) ==="
printf 'run  wall(s)   CPU%%    maxRSS(kB)\n'
for ((r = 1; r <= runs; r++)); do
  log="$outdir/bench_${label}_$r.log"
  cpu="$(grep -a "Percent of CPU" "$log" | awk -F': ' '{print $NF}')"
  rss="$(grep -a "Maximum resident" "$log" | awk -F': ' '{print $NF}')"
  printf '%-4s %-9s %-7s %s\n' "$r" "${walls[r-1]}" "$cpu" "$rss"
done

median() { printf '%s\n' "$@" | sort -n | awk '{a[NR]=$1} END { print (NR%2) ? a[(NR+1)/2] : (a[NR/2]+a[NR/2+1])/2 }'; }
med_wall="$(median "${walls[@]}")"
med_rss="$(median $(for ((r = 1; r <= runs; r++)); do grep -a "Maximum resident" "$outdir/bench_${label}_$r.log" | awk -F': ' '{print $NF}'; done))"
echo "MEDIAN wall: ${med_wall}s   MEDIAN maxRSS: ${med_rss} kB"
echo
echo "median [timing] stages (run with the median wall are in the logs above; full logs: $outdir/bench_${label}_*.log)"
echo "oracle: scripts/perf_oracle.sh <baseline_dir> $outdir/bench_${label}_1"
