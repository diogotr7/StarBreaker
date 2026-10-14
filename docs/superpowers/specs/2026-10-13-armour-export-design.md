# Armour export (design)

Status: implemented. Builds on the full-character work
(`docs/superpowers/plans/2026-10-15-full-character.md`, G1–G3). On
integration the single-piece path became main's sub-geometry tag
(`ExportOptions.geometry_tag`, `--geometry-tag Male|Female`, the app's
Armor mode) instead of the wearer gender, app category and gender choice
proposed here (see §1).

## Intent

Armour and clothing are common cosplay references, and people want to
build from them in Blender. StarBreaker should export them as well as it
exports ships: GLB and the decomposed `.blend` package, with materials,
rigged on the character skeleton.

Agreed with the user:

- The target is a **rigged set in Blender**, like ship exports. Making
  meshes print-ready (welding, closing holes, removing internal geometry)
  is out of scope.
- People **choose the pieces** (any mix of manufacturers). The game's
  loadout XMLs work too.
- Sets are exported **worn by a character**, through the existing
  character path. Someone who wants the armour alone deletes the head and
  body objects in Blender. (Under a full suit the clothing layers already
  leave the body out.)
- A dedicated armour-only rig, with no head or body, is **deferred**. It
  needs the same writer change as hiding the DNA head under a full helmet
  (a rig whose root mesh is not the head), so both are done together
  later.

Success:

1. Exporting a single armour piece with `entity export --geometry-tag`,
   or from the app's Armor export mode (its "Body variant" select), gives
   the piece itself, not its storage crate.
2. One CLI command exports a chosen set on a default male or female
   character, without needing a `.chf` file.

## What existed (before this work)

- **Entity export** (`pipeline/entity_export.rs:77`) reads only the
  record's root geometry path. For wearable items that is the dropped or
  stored prop (`crate_armor_arms_1_005x005x005.cgf` for
  `cds_armor_heavy_arms_01_01_01`). The worn `.skin` is in a sub-geometry
  node tagged with the gender (`Male` / `Female`). The app's export runs
  through the same path.
- **Character path:** `character::loadout::resolve_garments` already picks
  a gendered garment's `.skin` and material (`materialVariants`, else the
  node's, else the `.skin`'s own). `character::outfit::Outfit` dresses a
  CHF in a loadout with the clothing layers resolved.
- **App categories** (`app/src-tauri/src/commands.rs:861`
  `scan_categories`) are Ships / Ground Vehicles / Weapons / Other, split
  by path substrings. Armour falls into Other.

## Design

### 1. Single pieces export as worn

- **Worn rule.** `character::loadout::worn_geometry(db, record, gender:
  &str, context: &[String])` returns a record's worn geometry for a
  gender: the `.skin` of its sub-geometry node tagged exactly with that
  gender (else the other gender's, flagged `other_gender`), and its
  material from `materialVariants` resolved against `context` (gender +
  published tags), else the node's own. `resolve_garments` uses it.
- **Entity export (as built).** `ExportOptions` has `geometry_tag:
  Option<String>`. With a tag, entity export replaces the root geometry
  with the record's `SGeometryResourceParams` sub-geometry variant whose
  tag matches it (case-insensitive), keeping the root material when the
  variant has none (`pipeline/geometry_variant.rs`); no variant with that
  tag is an error. Without a tag every record exports as before, so ships
  and weapons are unaffected.
- **Skeleton.** The piece exports with the `.skin`'s own bone table at bind
  pose, as `skin export` does. Its bones are named like the player
  skeleton's, so it binds to any character rig by name.
- **CLI:** `entity export <item> --geometry-tag Male|Female`.
- **App:** the app's existing Armor export mode (`ExportView armorOnly`)
  passes its "Body variant" select (Male / Female / inventory model) as
  `geometry_tag`. The "Armour & Clothing" category and wearer-gender
  choice this design proposed were dropped in favour of it.

### 2. Sets through the character path

- **New command** `starbreaker armour export <out> --wear <item>...` (repeatable)
  with:
  - `--loadout <xml>`: added to the worn items;
  - `--gender male|female`: default male; picks the default wearer;
  - `--chf <file>`: any character as the wearer, instead of the default;
  - `--kind glb|package`: GLB is the default;
  - the existing animation flags (`--face-anim`, `--body-anim`,
    `--body-anim-clip`).
- **Default wearer.** The game's own default character,
  `Data\Libs\CharacterCustomizer\{masculine,feminine}default.chf`, read
  from the p4k.
- **Ports for `--wear` items.** Each item is worn on the port that accepts
  its `AttachDef.Type` (and subtype, when the port lists any) and carries
  the tags the port requires (`RequiredPortTags`). Sizes and the item's own
  `RequiredTags` are not checked (`wear::PortDef::accepts` says why). A
  piece with no mesh for the wearer's gender is reported. The ports searched are those of the body and of the
  items already worn, the way the engine equips. For example,
  `Char_Armor_Arms` goes on an undersuit's `Armor_Arms`, and
  `Char_Clothing_Torso_0` on the body's `Clothing_Torso_0`. With real
  ports, `HiddenParts` works for picked pieces too: an undersuit hides a
  shirt on `Clothing_Torso_0`. An item no worn port accepts is reported and
  skipped.
- **Under the hood** this builds `LoadoutEntry`s and calls the existing
  `chf_to_character_glb` / `export_character_package`. No new writer code.

### 3. Deferred

Armour-only export (no head or body), together with hiding the DNA head
under full helmets.

## Not in scope

- Print-ready meshes (STL/3MF, welding, closing holes, removing internal
  geometry).
- Item tint palettes (null on the armour inspected; the colour lives in
  the `.mtl` layers).
- Sets in the app's Character Builder.

## Testing

- **Unit:**
  - `worn_geometry` on synthetic sub-geometry: an exact gender node is
    used, else the other gender's; deeper tags are not; no gendered `.skin` returns `None`.
  - The port-matching rule.
- **p4k-gated:**
  - `entity export cds_armor_heavy_arms_01_01_01 --geometry-tag Male|Female`, GLB and package, for
    both genders: the output is the arms `.skin`, not the crate, and it is
    skinned to its bones.
  - A ship export is unchanged (vertex counts).
  - `armour export --wear` the marine heavy pieces gives the same meshes as
    the `marine_Armor_Heavy` loadout.
  - An undersuit plus a torso clothing item hides the clothing item.
- **Parity:** the single-piece GLB vs package is added as a fixture.
- **Blender:** check one set and one single piece.
