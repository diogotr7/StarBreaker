# Tint palette workflow — THE process

How to get StarBreaker **tint palettes / paint** right on an exported scene —
ships and socpaks — engine-faithfully and generically. This is the authoritative
how-to the `starbreaker-tint-palette` skill orchestrates. The append target for end-of-arc
retrospectives is `docs/tint-palette-process-improvements.md` (the ledger).

## §1 Strict rules (non-negotiable)

- **No hard-coded colour VALUES.** Never copy an RGB/RGBA into source, tests, or
  fallbacks (AGENTS: hard-coded game-data values are banned, "documented
  fallbacks" included). Every colour is DERIVED at run time from the DataCore
  `TintPaletteTree` record (decoded into `mtl::TintPalette`).
- **No name/asset gating.** Never branch on a specific object/mesh/ship/socpak/
  palette name or a magic field offset to "make this one right." Find the
  structural property (the per-object index field, the palette role index, the
  default-vs-override rule) and fix the rule for the whole category.
- **Fix the owning stage.** Wrong colour on screen → fix where the palette is
  DECODED or ASSIGNED (the chunk field, the index→path resolve, the role-slot
  map), not a draw-time tint nudge.
- **Generic across ships AND socpaks.** A fix must hold for both source kinds
  (they share `mtl::TintPalette` + `palettes.json`); they differ only in *where
  the palette assignment comes from* (§3).
- Captures/screens are imperfect (lighting, bloom, linear-vs-sRGB) — judge a
  colour **structurally** (which palette role, which decal stencil), not by a
  naked pixel pick.

## §2 Data model (decode this first — never assume)

A **tint palette** is a DataCore `TintPaletteTree` record, decoded into
`mtl::TintPalette` (`crates/starbreaker-3d/src/mtl.rs`): primary / secondary /
tertiary / glass finishes + a **decal** stencil (the manufacturer logo, e.g.
`gmni_logo_stencil` = Gemini). The decomposed export emits all distinct palettes
once into `Packages/<name>/palettes.json` (`{"palettes": [...], "version": 1}`);
every scene instance / object references one by **`palette_id`**.

- **Colour ROLE slots are not positional 1:1** — `BB_ColorStyle` role indices
  diverge (Bright=6 grey ≠ Base=0). When a role looks wrong, confirm the slot
  index against the enum, don't assume order (resolve a role by its enum
  index, never by position — ledger item 3).
- The **decal stencil is part of the palette identity** — a wrong logo (the
  Gemini incident) means the wrong *palette*, not a separate decal bug.

## §3 Where the palette assignment comes from (ships vs socpaks)

- **Ships:** the entity's `TintPaletteRef` selects the base palette; paint
  schemes / liveries swap it. Surfaced in `palettes.json` + `liveries.json`.
- **Socpaks:** each `IncludedObject` carries a **`tint_palette_index`**
  (`crates/starbreaker-3d/src/included_objects.rs`) — an index into the socpak's
  `tint_palette_paths` list. `0xFFFF` / out-of-range / `unknown3 != 0` ⇒ no
  override ⇒ the **default** palette. The index is decoded at chunk offset
  **+172 (word2)**, NOT +170 (word1, which is flags) — see ledger item 1.
- Resolution into a real palette happens in `pipeline/interiors.rs`
  (`resolve_interior_palette`, EXACT short-name match) and the per-object path
  flows `tint_palette_index → tint_palette_paths[i] → tint_palette_name →
  resolved TintPalette`.

## §4 The two modes

Decide which the request is before touching anything.

**Mode A — scene main palette (generalised).** Change the dominant palette for a
whole imported scene.
- Blender (post-import, fastest): select the package, run the **`Apply Palette`**
  operator (`starbreaker.apply_palette`, an EnumProperty over `palettes.json`
  ids) — or `Apply Paint` / `Apply Livery`. MCP/headless:
  `apply_palette_to_selected_package(context, palette_id)`.
- Exporter (when the WRONG palette is dominant at export): fix the default /
  dominant-palette resolution so the export emits the right `palette_id` as the
  scene default.

**Mode B — match specific objects.** Get individual objects' palettes right
(e.g. "these walls should be the blue brand palette, not default grey").
- This is almost always an **exporter** fix: the per-object
  `tint_palette_index → palette_id` decode/resolve (§3). VERIFY by PARSING the
  decoded indices against the expected per-object palettes — never eyeball one
  object and assume the field.
- The corrected per-object `palette_id` then rides through `palettes.json` and
  the object's instance record into Blender automatically.

## §5 The loop (per arc)

1. **Decode the source (§2/§3).** Dump the entity/socpak's `TintPaletteTree`
   (`search_records`/`datacore_record`), the `tint_palette_paths`, and the
   exported `palettes.json` + per-object `palette_id`s. PARSE them — don't assume.
2. **Diagnose.** Which mode (A/B)? What's structurally wrong (wrong index field,
   wrong role slot, default-vs-brand, wrong decal)? State the owning stage.
3. **Fix at the owning stage, generically (TDD).** Failing test first (e.g. the
   per-object index parses to the expected value), then the minimal structural
   fix. No colour literals, no name gates.
4. **Re-export.** Ships: `entity export <name> <root> --kind decomposed --lod 0
   --mip 0`. Socpaks: `socpak export <name> <root> --kind decomposed --lod 0
   --mip 0`. (`SC_DATA_P4K` auto-detected; debug binary is fine.)
5. **Re-import + verify (the trap that masks everything).** ALWAYS do a FRESH
   Blender import (`read_homefile`, then the import operator) — a stale scene
   shows the OLD palette. Then verify colours: parse `palettes.json` +
   per-object `palette_id`, and look at the objects (Blender MCP screenshot /
   the viewport). Confirm the expected palette role colours + decal.
6. **Closing re-check, then the retrospective (§6).**

## §6 Verification commands

The parse step of §5 (steps 1 and 5) is `scripts/tint_audit.py`: it reads
every package's `palettes.json` + `scene.json` and reports per-palette object
counts, unresolvable `palette_id`s, the ALL_SAME signature (ledger item 1), and
NO_OVERRIDE ids (ledger item 2). Exit status 1 means unresolvable ids or an
ALL_SAME signal.

```bash
# Parse exported palettes + per-object palette_id (Mode B sanity)
uv run python scripts/tint_audit.py <export_root_or_package_dir>
# Re-export (socpak example; Data.p4k is auto-detected)
./target/debug/starbreaker socpak export <name> <root> --kind decomposed --lod 0 --mip 0
# DataCore: find the tint palette tree / records
#   MCP: search_records("tint")  / datacore_record(<guid|name>)
```

Blender (deploy addon first if it changed): `rsync -a --delete
blender_addon/starbreaker_addon/ "$HOME/.config/blender/5.1/scripts/addons/starbreaker_addon/"`,
then in Blender `bpy.ops.wm.open_mainfile(filepath=".../scene.blend")` (the
load-post handler rebuilds the package materials) →
`bpy.ops.starbreaker.apply_palette(palette_id="...")` for Mode A.

## §7 Acceptance (bootstrap test)

A fresh agent can run the next arc from THIS doc + the ledger alone. Anything you
had to re-derive (a data location, a field offset, a record family, a don't-retry
trap) is a doc bug — fix it here in the same arc (that is the retrospective, §6 of
the skill).
