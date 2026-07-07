#!/usr/bin/env bash
set -euo pipefail

checks=(
  "crates/starbreaker-ui/src/ir_compose.rs::is_medical1_layout"
  "crates/starbreaker-ui/src/ir_compose.rs::medical_cyan_tint"
  "crates/starbreaker-ui/src/ir_compose.rs::Top_seperator"
  "crates/starbreaker-ui/src/ir_compose.rs::MedGelFillMeter"
  "crates/starbreaker-ui/src/ir_compose.rs::i_med_bioc_bottom-bar"
  "crates/starbreaker-ui/src/ir_compose.rs::BGDots"
  "crates/starbreaker-ui/src/ir_compose.rs::MainMenuCanvas"

  "crates/starbreaker-ui/src/compose.rs::base_animatedelements"
  "crates/starbreaker-ui/src/compose.rs::BGDots"
  "crates/starbreaker-ui/src/compose.rs::MainMenuCanvas"
  "crates/starbreaker-ui/src/compose.rs::s_bioc"
  "crates/starbreaker-ui/src/compose.rs::s_rsi"
  "crates/starbreaker-ui/src/compose.rs::s_aegs"

  "crates/starbreaker-ui/src/ui_ir.rs::nominal_font_size_from_label_style"
  "crates/starbreaker-ui/src/ui_ir.rs::BGDots"
  "crates/starbreaker-ui/src/ui_ir.rs::MainMenuCanvas"
  "crates/starbreaker-ui/src/ui_ir.rs::base_animatedelements"
)

# Non-empty-target precondition (alignment plan T5, ledger 61): if this list is
# ever emptied the loop below checks nothing and passes vacuously — the exact
# rot this guard exists to stop. Fail loudly on an empty check set.
source "$(dirname "${BASH_SOURCE[0]}")/lib/guard_assert.sh"
assert_nonzero_matches "${#checks[@]}" "hardcoding-guard checks (scripts/check_ui_hardcoding.sh)"

failed=0
for check in "${checks[@]}"; do
  file="${check%%::*}"
  marker="${check##*::}"
  # The monolithic ir_compose.rs / compose.rs / ui_ir.rs were decomposed into
  # directories; resolve `<file>.rs` to its `<dir>/` and search recursively.
  # A target that resolves to NEITHER a file nor a dir is a LOUD failure: a
  # silently-missing path means the guard checks nothing and a re-introduced
  # hard-code would pass unnoticed (the exact rot this guard exists to stop).
  target="$file"
  if [[ ! -e "$target" ]]; then
    dir="${file%.rs}"
    if [[ -d "$dir" ]]; then
      target="$dir"
    else
      echo "ERROR: hardcoding-guard target missing: '$file' (no '$dir/' either) — the guard is not checking '$marker'; fix the path after a rename/decompose." >&2
      failed=1
      continue
    fi
  fi
  if rg --fixed-strings --line-number -- "$marker" "$target" >/dev/null; then
    echo "Hardcoding marker found: $marker in $target"
    rg --fixed-strings --line-number -- "$marker" "$target" || true
    failed=1
  fi
done

if [[ "$failed" -ne 0 ]]; then
  echo "UI hardcoding guard failed."
  exit 1
fi

echo "UI hardcoding guard passed."
