#!/usr/bin/env bash
# Installs the pre-commit UI gate (alignment plan T4): copies the tracked hook
# body scripts/hooks/pre-commit-ui-gate into .git/hooks/pre-commit so a commit
# touching the UI renderer / shared scripts / MCP server is refused unless the
# last scripts/ui_check.sh run was genuinely green and recent (kills ledger-89,
# a piped ui_check exit code read as green).
#
# Usage:
#   bash scripts/install_ui_precommit.sh             # install for real
#   bash scripts/install_ui_precommit.sh --self-test # exercise the hook in a throwaway repo
#
# There is no pre-commit chaining: if an unexpected pre-commit hook already
# exists (and is not our own copy), install ABORTS for a human to resolve.
set -euo pipefail
# RESULT MARKER on failure (mirrors ui_check.sh / ui_arc_status.sh): a piped
# exit code is the filter's, not ours, so emit a distinct FAILED line on any
# non-zero exit. The OK line is printed by the body (rc still 0).
trap 'rc=$?; if [[ $rc -ne 0 ]]; then echo; echo "install_ui_precommit: FAILED (exit $rc) — see output above"; fi' EXIT

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
HOOK_SRC="${SCRIPT_DIR}/hooks/pre-commit-ui-gate"

install() {
  [[ -f "$HOOK_SRC" ]] || { echo "error: hook body missing: $HOOK_SRC" >&2; exit 1; }
  local git_dir target
  git_dir="$(git -C "$REPO_ROOT" rev-parse --absolute-git-dir)"
  target="${git_dir}/hooks/pre-commit"
  if [[ -e "$target" ]]; then
    if cmp -s "$HOOK_SRC" "$target"; then
      echo "pre-commit UI gate already installed (up to date): $target"
      echo "install_ui_precommit: OK (already installed)"
      return 0
    fi
    echo "error: an unexpected pre-commit hook already exists: $target" >&2
    echo "       refusing to overwrite (no chaining). Inspect it; if it is safe" >&2
    echo "       to remove, delete it and re-run, or merge our body in by hand:" >&2
    echo "         $HOOK_SRC" >&2
    exit 1
  fi
  cp "$HOOK_SRC" "$target"
  chmod +x "$target"
  echo "installed pre-commit UI gate -> $target"
  echo "install_ui_precommit: OK (installed)"
}

# --- self-test: drive the REAL hook via `git commit` in throwaway repos -------
# Each case is a fresh `git init` dir with our hook copied to its pre-commit and
# a synthetic marker; we stage a file and assert whether the commit is allowed.
_st_setup() {  # $1 = repo dir
  local dir="$1"
  git init -q "$dir"
  git -C "$dir" config user.email test@example.invalid
  git -C "$dir" config user.name "self-test"
  git -C "$dir" config commit.gpgsign false
  cp "$HOOK_SRC" "$dir/.git/hooks/pre-commit"
  chmod +x "$dir/.git/hooks/pre-commit"
  mkdir -p "$dir/crates/starbreaker-ui" "$dir/docs"
}

_st_marker() {  # $1 = repo dir, $2 = mode (green_fresh|stale|failed|none)
  local file="$1/.git/ui-check-marker" now
  now="$(date +%s)"
  case "$2" in
    green_fresh) printf 'ui_check: ALL GREEN\nhead=deadbeef\nepoch=%s\n' "$now" > "$file" ;;
    stale)       printf 'ui_check: ALL GREEN\nhead=deadbeef\nepoch=%s\n' "$((now - 4000))" > "$file" ;;
    failed)      printf 'ui_check: FAILED (exit 101)\nhead=deadbeef\nepoch=%s\n' "$now" > "$file" ;;
    none)        : ;;  # deliberately no marker file
  esac
}

_st_case() {  # $1 = label, $2 = marker mode, $3 = stage (ui|docs), $4 = expect (pass|block)
  local label="$1" mmode="$2" stage="$3" expect="$4"
  local dir; dir="$(mktemp -d "${TMPDIR:-/tmp}/ui_gate_st.XXXXXX")"
  _st_setup "$dir"
  _st_marker "$dir" "$mmode"
  if [[ "$stage" == docs ]]; then
    echo x > "$dir/docs/note.md"; git -C "$dir" add docs/note.md
  else
    echo x > "$dir/crates/starbreaker-ui/x.rs"; git -C "$dir" add crates/starbreaker-ui/x.rs
  fi
  local got
  if git -C "$dir" commit -q -m "self-test: $label" >/dev/null 2>&1; then got=pass; else got=block; fi
  rm -rf "$dir"
  if [[ "$got" == "$expect" ]]; then
    echo "  ok: $label -> $got"
    return 0
  fi
  echo "  FAIL: $label -> got '$got', expected '$expect'" >&2
  return 1
}

self_test() {
  [[ -f "$HOOK_SRC" ]] || { echo "error: hook body missing: $HOOK_SRC" >&2; exit 1; }
  echo "install_ui_precommit self-test (throwaway repos, real git commit):"
  _st_case "stale marker + UI staged"       stale       ui   block
  _st_case "green fresh marker + UI staged"  green_fresh ui   pass
  _st_case "docs-only staged (no marker)"    none        docs pass
  _st_case "FAILED marker + UI staged"       failed      ui   block
  echo "install_ui_precommit: OK (4 cases)"
}

case "${1:-}" in
  --self-test) self_test ;;
  -h|--help) sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
  "") install ;;
  *) echo "unknown argument: $1 (try --help)" >&2; exit 64 ;;
esac
