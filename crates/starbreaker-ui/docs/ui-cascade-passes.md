# UI style-cascade pass inventory (plan P4.1)

Every place a style entry is applied to the resolved scene today, in
execution order, with source container, palette sources, scope, and the
reference evidence that pinned it. This is the input inventory for the
Phase-4 selector-engine unification (plan completed; history in `crates/starbreaker-ui/docs/ui-process-improvements.md`);
code: `bb_resolve/engine_01.rs` (`apply_canvas_style_cascade`
and its caller `resolve_canvas_graph_inner`) + `bb_brand_apply/mod.rs` +
`pipeline/style_projection.rs`.

## Status: unified on `bb_style_engine` (plan P4.2–P4.4, 2026-06-13)

Every pass below now applies through the SINGLE selector engine
`bb_style_engine::apply` (a `StyleSheet` per container, tagged with its
`Tier`); the legacy per-entry-point wrappers and the identifier-prefix
sniff are deleted. This document remains the authoritative map of WHICH
sheets run and in what order — that order is unchanged, and the migration
was verified byte-identical on all frozen targets. The P4.4 re-audit kept
all four named survivors (see the plan): three are IR-compile-stage rules
the cascade engine does not own, and `RootGhost` is a name-pluck retained
pending a ghost-button reference.

## Execution order, per canvas resolve level

`resolve_canvas_graph_inner` runs bottom-up: children fully resolve (each
running this same sequence) before the parent's passes. At one level:

| # | Pass | Probe identifier | Source container | Palette (fills / chrome) | Scope |
|---|---|---|---|---|---|
| 1 | Widget-standard module sheets (during expansion) | `sk_<brand>_buttonsecondarystyles` etc. | `sk_<brand>_*styles` records | module sheet's own palette refs | expanded standard subtrees only |
| 2 | Deferred child late-state passes | the child's ORIGIN identifier (e.g. `s_drak_hud`) | child-canvas entries gated on then-PENDING state tags, collected at merge | empty palette (token-only — an unchanged token is a no-op) | that child's subtree |
| 3 | Widget-standard embedded | `widget-standard-embedded` | the expanded standards' own `embeddedStyles` | empty raw | whole scene (entries self-condition) |
| 4 | Style-link | linked record name | the canvas `style` record link — applied ONLY when no brand container resolves | the linked style record | whole scene |
| 5 | Shared styles | the shared record's name (`mfd_g_content`, `mfd_g_header`, `h_hud_g_controlhints_b`, …) | `defaultStyles.sharedStyles` URL | fills: brand container (or palette source / canvas); chrome: fetched brand `Style` record (`PaletteSources`) | whole scene |
| 6 | Brand container | `s_drak_hud`, `s_bioc`, `s_drak_env`, `orig`, … | selected `brandStyles[]` (manufacturer match) | fills: brand container; chrome: brand `Style` record | whole scene; **the TEXT-FORMAT route lives ONLY here** (see below) |
| 7 | Embedded styles | `embeddedStyles` | the canvas's `embeddedStyles` | the canvas / inherited palette source | whole scene |
| 8 | Inline-only finishing pass | `inline` (was `?` pre-P4.3) | empty entry list | shared fills | guarantees node `inlineStyles` apply on canvases with no other containers |
| 9 | Scrollbar module sheet | `sk_<brand>_scrollbarstyles` | `apply_scrollbar_modular_styles` | module chrome palette | expanded scrollbar standards only |
| 10 | `exportNode=false` subtree deactivation | — (not styling) | authored node flag | — | editor-only subtrees (plan P5.1) |

Node `inlineStyles` are applied LAST inside **every** entry pass above (the
per-pass loop ends with the node's own inline entries), so an inline value
wins within each pass; pass order decides across passes.

Post-resolve, in the pipeline (`project_canvas_style_entries`,
`pipeline/style_projection.rs`): the ROOT (binding frame) canvas's
`defaultStyles.entries` are applied to the resolved scene, followed by a
re-application of the root brand container. **Flag for P4.4 re-audit**:
this is the one place defaultStyles entries run at all (see below), scoped
to the root canvas only.

### Pending-state filtering

Entries gated on a PENDING state tag (producing chain unresolvable at this
level — a parent-injected param) are filtered out of passes 4–7 and re-run
later as pass 2 of the PARENT level, against the then-resolved tags.
Evidence: the offline chiclet's glow (mis-styled when evaluated with the
tag assumed absent).

### defaultStyles are editor-time (NOT a cascade stage)

`defaultStyles.entries` do not run during the cascade. Evidence (comment at
`apply_canvas_style_cascade`): the annunciator `CornerRadius` (radius 30)
lives only there and the in-game chiclets are square; the power
`System Icon Color` exists in defaultStyles + misc/orig brand containers
but NOT drak's, and the in-game drak system icons render the SVGs' own
white. A previous cascade-BASE application of defaultStyles rounded the
chiclets and tinted the white icons — both away from the reference.
(`defaultStyles.sharedStyles` — the URL next to the entries — IS consumed,
as pass 5.)

### The text-format route (Brand tier only)

Gated on `Tier::Brand` since P4.3 (was an `s_*` identifier-prefix sniff). A
Parent-wrapped entry on a brand container whose conditions select a
TEXTFIELD styles the field's TEXT FORMAT (FontSize/FillColor), not the
widget. The applied FontSize sets the `__EntryFontSize` marker, which is
the ONLY thing that outranks the named-style table in font resolution — a
literal widget match does not (T3 counterexample; commit `07c821a83`). An
inline `FontSize` sets `__InlineFontSize` and outranks the brand-table
standard. Probe: `BB_TEXT_FORMAT_PROBE=1` (TFPROBE = route applications;
TFPROBE-NORMAL = normal-route FontSize carriers).

## Probe-order acceptance (BB_A3_STYLE_PROBE=1, 2026-06-12)

Observed per-level identifier sequence — power render
(`Screen_Left_Lower_RTT`, LOD0): small canvases print
`embeddedStyles → ?`; the brand-styled canvases print
`s_drak_hud → embeddedStyles → ? → sk_drak_hud_buttonsecondarystyles` —
matching passes 6→7→8 then 1/9 of the NEXT enclosing level. Medical render
(LOD1) additionally interleaves `widget-standard-embedded` before a level's
`embeddedStyles` (pass 3 before 7 ✓) and prints the shared records
(`mfd_g_*`, `h_hud_g_controlhints_b`), other-brand containers (`s_bioc`,
`orig`, `s_drak_env`, `s_aegs_env`) and both `sk_bioc_*` module sheets.
Distinct identifiers observed across both renders: `?`, `embeddedStyles`,
`h_hud_g_controlhints_b`, `mfd_g_content`, `mfd_g_emissions`,
`mfd_g_generalweaponinfo`, `mfd_g_header`, `mfd_g_targetstatus`,
`mfd_g_weaponinfoflyout`, `orig`, `s_aegs_env`, `s_bioc`, `s_drak_env`,
`s_drak_hud`, `sk_aegs_env_buttonsecondarystyles`,
`sk_bioc_buttonsecondarystyles`, `sk_bioc_linearprogressmeterstyles`,
`sk_drak_hud_buttonsecondarystyles`, `sk_drak_hud_scrollbarstyles`,
`widget-standard-embedded` — every one maps to a pass row above.
