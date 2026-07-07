# B4 Linear-Light Compositing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move every blend/composite in the starbreaker-ui renderer from sRGB space to linear light — renderer-wide, no carve-outs — then re-freeze all 15 gold/platinum baselines.

**Architecture:** Strategy **A** (fixed by `.superpowers/sdd/B4-research.md`): per-site manual linear composite into the existing u8-premultiplied-sRGB `tiny_skia::Pixmap`, generalising the landed `blit_white_mask_overlay_linear` carve-out (`ir_compose/engine_01.rs:923`). Storage stays u8 sRGB, so `pixmap_to_rgba_image` and `encode_png` are UNCHANGED. NOT strategy B (no linear framebuffer, no tiny-skia replacement). The two channel helpers move to a new pure `crate::colour` module gaining a u8→linear LUT plus two shared blend primitives; each blend site (flat/shape fills, texture blits, manual clip composites, the ttf + swf glyph rasterisers) routes through them. The white-mask carve-out is folded into the now-linear general blit. The IR-geometry snapshot must stay byte-identical (blending changes colour, never layout).

**Tech Stack:** Rust, `tiny-skia` (u8 premultiplied Pixmap), `image::RgbaImage` (straight-alpha u8), `rusttype`/SWF glyph rasterisers. Crate `starbreaker-ui`. Branch `feature/ui`.

## Global Constraints

- **TDD:** failing test first, verify it fails, minimal implementation, verify pass, commit. For per-site tasks the failing test asserts the KNOWN linear blend result (and that it differs from the sRGB result).
- **No hard-coded game-data values** in production code or fixtures (AGENTS.md). Test colours are visibly synthetic (pure white over pure black); the one real reference triple (annunciator chiclet edge) already lives in the existing `white_mask_overlay_composites_in_linear_light` test and is retargeted, not newly introduced.
- **One commit per task on `feature/ui`.** No new branches/worktrees (memory: stay-on-feature-ui-branch). No `Co-authored-by` trailer. Never name the maintainer — prose says "the owner", freeze uses `--approver owner`.
- **Every touched `.rs` keeps an accurate `//!` module header and `///` doc comments** describing the current responsibility (AGENTS.md). The new `colour.rs` needs a `//!` header.
- **`--full` requires a release rebuild + LOD0 export:** `cargo build --release` then `entity export ... --lod 0 --mip 0 --materials all`. LOD1 CULLS the small cockpit HUD screens, so a re-freeze off LOD1 would freeze stale/missing PNGs.
- **NO `/graphify . --update`** — the post-commit hook re-extracts changed code automatically.
- **Perf:** the render stage is ~1% of wall (`crates/starbreaker-ui/docs/ui-perf-baseline.md`, stale B0 serial run). The u8→linear LUT keeps the sRGB→linear direction off `powf` for straight-alpha reads. Re-measure at re-freeze with the doc's method (`SB_UI_TIMING=1 … RAYON_NUM_THREADS=1`); watch the render + encode timers.
- **Intermediate `--full` RED is EXPECTED** on the 15 targets from Task 2 through Task 6 (every composited/AA edge pixel moves, so every whole-image sha256 changes). Per-task validation therefore runs the **TDD tier** `bash scripts/ui_check.sh` (visual guards skipped) + the new unit test, which stay GREEN. The geometry IR snapshot staying byte-identical is the guard that only colour moved. `--full` goes GREEN again only after the Task 7 re-freeze.

---

## File Structure

- **Create `crates/starbreaker-ui/src/colour.rs`** — pure colour-space module (no `tiny_skia`, no `image` types in its signatures beyond `[u8;4]`/`[u8;3]`): `srgb_channel_to_linear`, `linear_channel_to_srgb` (moved from `engine_01.rs:900/908`), the u8→linear LUT `u8_to_linear`, and three shared blend primitives `blend_straight_linear`, `blend_premul_linear`, `blend_premul_add_linear`. Reachable from both `ir_compose/` and `text/` (siblings), so it lives at crate root, not under `ir_compose/`.
- **Declare it in `crates/starbreaker-ui/src/lib.rs`** — add `pub(crate) mod colour;` alongside the other module declarations (near line 35).
- **Modify `crates/starbreaker-ui/src/ir_compose/fill_primitives.rs`** — add `fill_linear` scratch-composite helper; route the 3 fill fns through it.
- **Modify `crates/starbreaker-ui/src/ir_compose/engine_01.rs`** — delete the two moved channel helpers (import from `crate::colour`); route the widget-circle fill / separator stroke through `fill_linear`; rewrite `blit_atlas_image_tinted_with_mode` to a linear texel loop; (Task 6) delete `blit_white_mask_overlay_linear` and fold its call site.
- **Modify `crates/starbreaker-ui/src/ir_compose/engine_02.rs`** — linearise `composite_clip_region_pixmap` + `composite_clip_region_image`; route `draw_ir_polygon`'s fill through `fill_linear`; retarget the white-mask test (Task 6).
- **Modify `crates/starbreaker-ui/src/text/ttf_draw.rs`** — glyph coverage closure → `blend_straight_linear`.
- **Modify `crates/starbreaker-ui/src/text/swf_draw.rs`** — `blend_pixmap_onto_image` → `blend_premul_linear`.
- **Modify docs (Task 7):** `docs/ui-architecture-runbook.md:413-423`, `docs/ui-clipper-parity-handoff.md:278-282`, `docs/ui-process-improvements.md` (ledger 15/18), `docs/ui-reference.md` ("linear-light compositing gap" cells).

**Pre-flight (before Task 1, no commit):** confirm the working tree is on `feature/ui`, B1–B3 are landed, and `bash scripts/ui_check.sh --full` is GREEN on a fresh export (parent-plan Step 1). If `--full` is not green for an unrelated reason, STOP — the re-freeze delta table at Task 7 needs a clean pre-migration baseline.

```bash
git -C "$(git rev-parse --show-toplevel)" branch --show-current   # expect: feature/ui
bash scripts/ui_check.sh --full                                    # expect: "ui_check: ALL GREEN"
```

---

## Task 1: Shared `colour` module — channel helpers, u8→linear LUT, blend primitives

**Files:**
- Create: `crates/starbreaker-ui/src/colour.rs`
- Modify: `crates/starbreaker-ui/src/lib.rs` (add `pub(crate) mod colour;`)
- Modify: `crates/starbreaker-ui/src/ir_compose/engine_01.rs:900-914` (delete the two fns; import from `crate::colour`)
- Test: unit tests inside `crates/starbreaker-ui/src/colour.rs`

**Interfaces:**
- Produces (all `pub(crate)` in `crate::colour`):
  - `fn srgb_channel_to_linear(c: f32) -> f32` — piecewise EOTF (moved verbatim).
  - `fn linear_channel_to_srgb(l: f32) -> f32` — inverse OETF (moved verbatim).
  - `fn u8_to_linear(v: u8) -> f32` — LUT-backed sRGB→linear for u8 inputs; exactly equals `srgb_channel_to_linear(v as f32 / 255.0)`.
  - `fn blend_straight_linear(dst: &mut [u8; 4], src_rgb: [u8; 3], src_a: f32)` — straight-alpha source-over in linear. `src_a` in 0..1 folds coverage × colour-alpha.
  - `fn blend_premul_linear(dst: &mut [u8; 4], src: [u8; 4])` — premultiplied source-over in linear (the carve-out arithmetic, generalised).
  - `fn blend_premul_add_linear(dst: &mut [u8; 4], src: [u8; 4])` — premultiplied additive (`BlendMode::Plus`) in linear.
- This task is a **pure refactor for the two moved fns** (same math, same callers via the carve-out) plus **new, not-yet-wired** blend primitives. `blit_white_mask_overlay_linear` and every other site keep behaving identically until Tasks 2–6 route through the new primitives.

- [ ] **Step 1: Write the new module with its failing tests**

Create `crates/starbreaker-ui/src/colour.rs`:

```rust
//! Colour-space conversions and linear-light blend primitives shared by the IR
//! compositor (`ir_compose/`) and the text glyph rasterisers (`text/`).
//!
//! The engine composites in LINEAR light; the u8-premultiplied-sRGB framebuffer
//! (`tiny_skia::Pixmap`) and the straight-alpha `image::RgbaImage` both blend in
//! their STORED sRGB space unless a site converts. These helpers do the
//! per-pixel sRGB→linear→blend→sRGB round trip. `u8_to_linear` is a 256-entry
//! LUT so the sRGB→linear direction (whose inputs are always u8) costs a lookup
//! instead of a `powf`.
//!
//! Key items: `srgb_channel_to_linear` / `linear_channel_to_srgb` (channel
//! EOTF/OETF), `u8_to_linear` (LUT), and the three blend primitives
//! `blend_straight_linear`, `blend_premul_linear`, `blend_premul_add_linear`.

use std::sync::OnceLock;

/// sRGB electro-optical transfer function (gamma-decode) for a single 0..1
/// channel: piecewise 12.92 toe + 2.4 gamma.
pub(crate) fn srgb_channel_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Inverse of [`srgb_channel_to_linear`] (gamma-encode) for a single 0..1 linear
/// channel.
pub(crate) fn linear_channel_to_srgb(l: f32) -> f32 {
    if l <= 0.003_130_8 {
        12.92 * l
    } else {
        1.055 * l.powf(1.0 / 2.4) - 0.055
    }
}

/// sRGB→linear for a u8 channel, via a 256-entry LUT (built once). Every blend
/// input that is a stored u8 sRGB byte goes through here, avoiding a per-pixel
/// `powf`.
pub(crate) fn u8_to_linear(v: u8) -> f32 {
    static LUT: OnceLock<[f32; 256]> = OnceLock::new();
    LUT.get_or_init(|| std::array::from_fn(|i| srgb_channel_to_linear(i as f32 / 255.0)))[v as usize]
}

/// Straight-alpha source-over in LINEAR light. `dst` is straight-alpha u8 sRGB
/// (an `image::RgbaImage` pixel's `.0`); `src_rgb` is the straight u8 sRGB
/// source colour; `src_a` in 0..1 already folds coverage × source alpha.
pub(crate) fn blend_straight_linear(dst: &mut [u8; 4], src_rgb: [u8; 3], src_a: f32) {
    let src_a = src_a.clamp(0.0, 1.0);
    if src_a <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let out_a = src_a + da * (1.0 - src_a);
    if out_a <= 0.0 {
        return;
    }
    for c in 0..3 {
        let s_lin = u8_to_linear(src_rgb[c]);
        let d_lin = u8_to_linear(dst[c]);
        let out_lin = (s_lin * src_a + d_lin * da * (1.0 - src_a)) / out_a;
        dst[c] = (linear_channel_to_srgb(out_lin.clamp(0.0, 1.0)) * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Premultiplied source-over in LINEAR light. Both `dst` and `src` are
/// premultiplied u8 sRGB (a `tiny_skia::Pixmap` pixel). Generalises the landed
/// white-mask carve-out (`ir_compose/engine_01.rs` history) to any source.
pub(crate) fn blend_premul_linear(dst: &mut [u8; 4], src: [u8; 4]) {
    let sa = src[3] as f32 / 255.0;
    if sa <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let out_a = sa + da * (1.0 - sa);
    if out_a <= 0.0 {
        return;
    }
    for c in 0..3 {
        let s_lin = srgb_channel_to_linear(((src[c] as f32 / 255.0) / sa).clamp(0.0, 1.0));
        let d_lin = if da > 0.0 {
            srgb_channel_to_linear(((dst[c] as f32 / 255.0) / da).clamp(0.0, 1.0))
        } else {
            0.0
        };
        let out_lin = (s_lin * sa + d_lin * da * (1.0 - sa)) / out_a;
        dst[c] = (linear_channel_to_srgb(out_lin.clamp(0.0, 1.0)) * out_a * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

/// Premultiplied additive (`tiny_skia::BlendMode::Plus`) in LINEAR light — used
/// by glow layers (hologram, radar disc, `*_glow.tif`). Sums premultiplied
/// linear channels, clamped to the output alpha.
pub(crate) fn blend_premul_add_linear(dst: &mut [u8; 4], src: [u8; 4]) {
    let sa = src[3] as f32 / 255.0;
    let da = dst[3] as f32 / 255.0;
    let out_a = (sa + da).min(1.0);
    if out_a <= 0.0 {
        return;
    }
    for c in 0..3 {
        let s_pl = if sa > 0.0 {
            srgb_channel_to_linear(((src[c] as f32 / 255.0) / sa).clamp(0.0, 1.0)) * sa
        } else {
            0.0
        };
        let d_pl = if da > 0.0 {
            srgb_channel_to_linear(((dst[c] as f32 / 255.0) / da).clamp(0.0, 1.0)) * da
        } else {
            0.0
        };
        let out_pl = (s_pl + d_pl).min(out_a);
        let out_straight = (out_pl / out_a).clamp(0.0, 1.0);
        dst[c] = (linear_channel_to_srgb(out_straight) * out_a * 255.0)
            .round()
            .clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lut_matches_powf_helper_for_all_bytes() {
        for v in 0u16..=255 {
            let v = v as u8;
            assert_eq!(
                u8_to_linear(v),
                srgb_channel_to_linear(v as f32 / 255.0),
                "LUT[{v}] must equal the powf helper exactly"
            );
        }
        assert_eq!(u8_to_linear(0), 0.0);
        assert!((u8_to_linear(255) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn channel_round_trip_is_near_identity() {
        for v in 0u16..=255 {
            let c = v as f32 / 255.0;
            let back = linear_channel_to_srgb(srgb_channel_to_linear(c));
            assert!((back - c).abs() < 1e-4, "round trip drift at {c}: {back}");
        }
    }

    // 50% coverage of white over OPAQUE black: linear blend lands ~188, NOT the
    // sRGB midpoint 128. This is the whole point of the migration.
    #[test]
    fn blend_straight_linear_white_over_black_is_linear_not_srgb() {
        let mut dst = [0u8, 0, 0, 255];
        blend_straight_linear(&mut dst, [255, 255, 255], 0.5);
        assert!(
            (dst[0] as i32 - 188).abs() <= 2 && dst[3] == 255,
            "linear 50% white over black expected ~188, got {dst:?}"
        );
        assert!((dst[0] as i32 - 128).abs() > 20, "must NOT be the sRGB midpoint 128");
    }

    #[test]
    fn blend_premul_linear_half_white_over_black_is_linear() {
        // premultiplied white at alpha 0.5 => (128,128,128,128) over opaque black.
        let mut dst = [0u8, 0, 0, 255];
        blend_premul_linear(&mut dst, [128, 128, 128, 128]);
        assert!(
            (dst[0] as i32 - 188).abs() <= 3 && dst[3] == 255,
            "linear premul 50% white over black expected ~188, got {dst:?}"
        );
    }

    #[test]
    fn blend_premul_add_linear_sums_in_linear() {
        // Additive of premultiplied opaque mid-grey onto itself: linear(0.502)
        // + linear(0.502) ≈ 0.442 -> srgb ≈ 0.70 -> ~178. A naive sRGB byte sum
        // would clamp to 128+128=255 immediately.
        let mut dst = [128u8, 128, 128, 255];
        blend_premul_add_linear(&mut dst, [128, 128, 128, 255]);
        assert!(
            dst[0] < 255 && dst[0] > 150,
            "additive-in-linear grey+grey expected ~178, got {dst:?}"
        );
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail (module not yet declared)**

Run: `cargo test -p starbreaker-ui --lib colour:: 2>&1 | tail -20`
Expected: FAIL — `file not found for module \`colour\`` / `cannot find` until Step 3 wires the module in.

- [ ] **Step 3: Declare the module and delete the two moved helpers from `engine_01.rs`**

In `crates/starbreaker-ui/src/lib.rs`, add next to the other `pub mod` lines (near line 35):

```rust
pub(crate) mod colour;
```

In `crates/starbreaker-ui/src/ir_compose/engine_01.rs`, DELETE the two functions at lines 900-914 (`srgb_channel_to_linear`, `linear_channel_to_srgb`) and add an import near the top of the file (with the other `use` lines):

```rust
use crate::colour::{linear_channel_to_srgb, srgb_channel_to_linear};
```

`blit_white_mask_overlay_linear` (still at `:923`) now references the imported helpers unchanged. Leave it — it is folded in Task 6.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p starbreaker-ui --lib colour:: 2>&1 | tail -20`
Expected: PASS — 5 tests. Also confirm the crate still builds: `cargo build -p starbreaker-ui 2>&1 | tail -5` (the carve-out compiles against the imported helpers).

- [ ] **Step 5: TDD-tier check + commit**

Run: `bash scripts/ui_check.sh 2>&1 | tail -5`
Expected: `ui_check: ALL GREEN` (the moved helpers are byte-identical; no baseline moves).

```bash
git add crates/starbreaker-ui/src/colour.rs crates/starbreaker-ui/src/lib.rs crates/starbreaker-ui/src/ir_compose/engine_01.rs
git commit -m "ui(B4): shared colour module — channel helpers, u8→linear LUT, linear blend primitives"
```

---

## Task 2: Flat/shape fills → linear scratch composite

Every tiny-skia shape draw (fills + the widget-circle fill + the separator stroke + the power-pip polygon fill) currently blends onto `dst` in stored sRGB. Migrate all of them to: draw into a bounds-sized transparent scratch pixmap (sRGB), then composite the scratch onto `dst` in linear via `blend_premul_linear` / `blend_premul_add_linear`, honouring `SourceOver` / `Plus`. `to_skia_color` stays sRGB — linearisation happens at composite time (matching the carve-out, which converts at blit time, not at token resolution). Additive fills (`node_colour_blend_mode` → `Plus`, `engine_01.rs:1015/1020/2739`) are covered here because `fill_linear` honours `Plus`.

**Files:**
- Modify: `crates/starbreaker-ui/src/ir_compose/fill_primitives.rs` (add `fill_linear`; route the 3 fill fns)
- Modify: `crates/starbreaker-ui/src/ir_compose/engine_01.rs:2684` (widget circle `.fill_path`), `:2803-2812` (separator `.stroke_path`)
- Modify: `crates/starbreaker-ui/src/ir_compose/engine_02.rs:263-` (`draw_ir_polygon` `.fill_path` at `:307`)
- Test: `crates/starbreaker-ui/src/ir_compose/fill_primitives.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `crate::colour::{blend_premul_linear, blend_premul_add_linear}` (Task 1).
- Produces (`pub(crate)` in `fill_primitives`, reachable in `engine_01`/`engine_02` via `ir_compose::mod`'s `pub(crate) use fill_primitives::*`):
  - `fn fill_linear(dst: &mut Pixmap, bounds: TskRect, blend_mode: BlendMode, draw: impl FnOnce(&mut Pixmap, Transform))`

- [ ] **Step 1: Write the failing test**

Add to `crates/starbreaker-ui/src/ir_compose/fill_primitives.rs` `#[cfg(test)]`:

```rust
#[cfg(test)]
mod linear_fill_tests {
    use super::*;
    use tiny_skia::{BlendMode, Color, Pixmap, Rect as TskRect};

    // A 50%-alpha white rect over opaque black must land ~188 (linear), not 128 (sRGB).
    #[test]
    fn fill_rect_composites_in_linear_light() {
        let mut pm = Pixmap::new(4, 4).unwrap();
        pm.fill(Color::from_rgba8(0, 0, 0, 255));
        let rect = TskRect::from_xywh(0.0, 0.0, 4.0, 4.0).unwrap();
        fill_rect_ts_with_mode(&mut pm, rect, [1.0, 1.0, 1.0, 1.0], 0.5, BlendMode::SourceOver);
        let px = pm.pixel(1, 1).unwrap();
        assert!(
            (px.red() as i32 - 188).abs() <= 3,
            "linear 50% white fill over black expected ~188, got {}",
            px.red()
        );
        assert!((px.red() as i32 - 128).abs() > 20, "must not be sRGB 128");
    }

    #[test]
    fn additive_fill_sums_in_linear() {
        let mut pm = Pixmap::new(4, 4).unwrap();
        pm.fill(Color::from_rgba8(128, 128, 128, 255));
        let rect = TskRect::from_xywh(0.0, 0.0, 4.0, 4.0).unwrap();
        fill_rect_ts_with_mode(&mut pm, rect, [0.502, 0.502, 0.502, 1.0], 1.0, BlendMode::Plus);
        let px = pm.pixel(1, 1).unwrap();
        assert!(px.red() < 255 && px.red() > 150, "additive-in-linear expected ~178, got {}", px.red());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p starbreaker-ui --lib fill_rect_composites_in_linear_light 2>&1 | tail -20`
Expected: FAIL — `fill_rect_ts_with_mode` still blends in sRGB, `px.red()` ≈ 128.

- [ ] **Step 3: Add `fill_linear` and route the fills through it**

Add to `crates/starbreaker-ui/src/ir_compose/fill_primitives.rs` (imports at top gain `use crate::colour::{blend_premul_add_linear, blend_premul_linear};` and `Transform` is already imported):

```rust
/// Render `draw` (a tiny-skia shape fill/stroke) into a transparent scratch
/// pixmap sized to `bounds` (padded for AA / stroke bleed), then composite the
/// scratch onto `dst` in LINEAR light, honouring `SourceOver` / `Plus`. The
/// closure receives a translation `Transform` mapping `bounds`'s origin to the
/// scratch's (0,0) so callers draw in absolute coordinates. Generalises the
/// white-mask carve-out to arbitrary tiny-skia draws so every AA edge blends in
/// linear, matching the engine.
pub(crate) fn fill_linear(
    dst: &mut Pixmap,
    bounds: TskRect,
    blend_mode: BlendMode,
    draw: impl FnOnce(&mut Pixmap, Transform),
) {
    const PAD: f32 = 2.0; // AA / stroke half-width bleed
    let x0 = (bounds.x() - PAD).floor().max(0.0) as u32;
    let y0 = (bounds.y() - PAD).floor().max(0.0) as u32;
    let x1 = ((bounds.x() + bounds.width() + PAD).ceil().max(0.0) as u32).min(dst.width());
    let y1 = ((bounds.y() + bounds.height() + PAD).ceil().max(0.0) as u32).min(dst.height());
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let (sw, sh) = (x1 - x0, y1 - y0);
    let Some(mut scratch) = Pixmap::new(sw, sh) else {
        return;
    };
    draw(&mut scratch, Transform::from_translate(-(x0 as f32), -(y0 as f32)));

    let sd = scratch.data();
    let dw = dst.width();
    let dd = dst.data_mut();
    for ly in 0..sh {
        for lx in 0..sw {
            let si = ((ly * sw + lx) * 4) as usize;
            let s = [sd[si], sd[si + 1], sd[si + 2], sd[si + 3]];
            if s[3] == 0 && blend_mode != BlendMode::Plus {
                continue;
            }
            let di = (((y0 + ly) * dw + (x0 + lx)) * 4) as usize;
            let mut d = [dd[di], dd[di + 1], dd[di + 2], dd[di + 3]];
            match blend_mode {
                BlendMode::Plus => blend_premul_add_linear(&mut d, s),
                _ => blend_premul_linear(&mut d, s),
            }
            dd[di..di + 4].copy_from_slice(&d);
        }
    }
}
```

Rewrite the three fill fns to draw into the scratch. `fill_rect_ts_with_mode`:

```rust
pub(crate) fn fill_rect_ts_with_mode(
    pixmap: &mut Pixmap,
    rect: TskRect,
    rgba: [f32; 4],
    alpha: f32,
    blend_mode: BlendMode,
) {
    fill_linear(pixmap, rect, blend_mode, |scratch, tf| {
        let mut paint = Paint::default();
        paint.set_color(to_skia_color(rgba, alpha));
        paint.blend_mode = BlendMode::SourceOver; // into transparent scratch
        paint.anti_alias = false;
        scratch.as_mut().fill_rect(rect, &paint, tf, None);
    });
}
```

`fill_rounded_rect_ts_with_mode` — its `bounds` is `rect` (the path is inside it); route the `fill_path` branch the same way (the square fallback already calls the migrated `fill_rect_ts_with_mode`):

```rust
pub(crate) fn fill_rounded_rect_ts_with_mode(
    pixmap: &mut Pixmap,
    rect: TskRect,
    radius: f32,
    rgba: [f32; 4],
    alpha: f32,
    blend_mode: BlendMode,
) {
    let Some(path) = rounded_rect_path(rect, radius) else {
        fill_rect_ts_with_mode(pixmap, rect, rgba, alpha, blend_mode);
        return;
    };
    fill_linear(pixmap, rect, blend_mode, |scratch, tf| {
        let mut paint = Paint::default();
        paint.set_color(to_skia_color(rgba, alpha));
        paint.blend_mode = BlendMode::SourceOver;
        paint.anti_alias = true;
        scratch.as_mut().fill_path(&path, &paint, tiny_skia::FillRule::Winding, tf, None);
    });
}
```

`fill_corner_geometry_ts_with_mode` — identical shape (bounds = `rect`, `fill_path` with the per-corner `path`); apply the same `fill_linear` wrapping.

Then in `engine_01.rs`, wrap the two direct pixmap draws with `fill_linear` (bounds from the path's `.bounds()`; stroke width is already inside the PAD budget for thin separators, but pass a bounds inflated by the stroke width for safety):
- `:2684` widget-circle `.fill_path` → `fill_linear(pixmap, path.bounds(), BlendMode::SourceOver, |s, tf| s.as_mut().fill_path(&path, &paint, FillRule::Winding, tf, None))`.
- `:2803-2812` separator `.stroke_path` → compute `let b = path.bounds();` inflate by `stroke.width` and pass to `fill_linear`, drawing `s.as_mut().stroke_path(&path, &paint, &stroke, tf, None)` inside.

And in `engine_02.rs` `draw_ir_polygon` (`:307` `.fill_path`) → wrap the same way (bounds = `path.bounds()`, `BlendMode::SourceOver`).

In each case `paint.blend_mode` for the scratch draw is `SourceOver` (drawing onto transparent), and the real `blend_mode` is passed to `fill_linear`.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p starbreaker-ui --lib linear_fill_tests 2>&1 | tail -20`
Expected: PASS — both tests.

- [ ] **Step 5: Confirm the diff is colour-only, TDD-tier check, commit**

Run: `git diff --stat` — confirm changes are confined to `fill_primitives.rs`, `engine_01.rs`, `engine_02.rs` blend/draw code, no rect/layout maths.
Run: `bash scripts/ui_check.sh 2>&1 | tail -5`
Expected: `ui_check: ALL GREEN` (visual guards skipped in TDD tier; the geometry-space unit tests and the whole lib suite pass).

```bash
git add crates/starbreaker-ui/src/ir_compose/fill_primitives.rs crates/starbreaker-ui/src/ir_compose/engine_01.rs crates/starbreaker-ui/src/ir_compose/engine_02.rs
git commit -m "ui(B4): flat/shape fills composite in linear light via scratch"
```

---

## Task 3: Texture/image blits → linear texel loop

Rewrite `blit_atlas_image_tinted_with_mode` (`engine_01.rs:2842`) so the per-texel premultiply happens in LINEAR light (folds the texture sRGB ingest per research §2: convert each texel's RGB at the blit, NOT in the atlas cache — the cache stays sRGB so re-tinting stays correct). This replaces the sRGB build-loop + `draw_pixmap` entirely with one linear composite loop, honouring `SourceOver` / `Plus`. Covers hologram (`:229`), radar disc (`:294`), icons/logos, tinted images, and the additive image path (`image_blend_mode_for_node` → `Plus`).

**Files:**
- Modify: `crates/starbreaker-ui/src/ir_compose/engine_01.rs:2842-2879` (`blit_atlas_image_tinted_with_mode`)
- Test: `crates/starbreaker-ui/src/ir_compose/engine_01.rs` or `engine_02.rs` `#[cfg(test)]` (wherever the blit tests already live)

**Interfaces:**
- Consumes: `crate::colour::{linear_channel_to_srgb, srgb_channel_to_linear, u8_to_linear}` (import already added in Task 1 for the first two; add `u8_to_linear`).
- Signature UNCHANGED: `fn blit_atlas_image_tinted_with_mode(pixmap: &mut Pixmap, img: &RgbaImage, dx: i32, dy: i32, tint: [f32; 4], alpha: f32, blend_mode: BlendMode)`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn blit_atlas_tinted_composites_in_linear_light() {
    let mut pm = Pixmap::new(2, 2).unwrap();
    pm.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
    let mut img = RgbaImage::new(2, 2);
    for p in img.pixels_mut() {
        *p = image::Rgba([255, 255, 255, 255]); // opaque white texel
    }
    // white tint, 50% node alpha over opaque black => linear ~188, sRGB ~128.
    blit_atlas_image_tinted_with_mode(&mut pm, &img, 0, 0, [1.0, 1.0, 1.0, 1.0], 0.5, BlendMode::SourceOver);
    let px = pm.pixel(0, 0).unwrap();
    assert!((px.red() as i32 - 188).abs() <= 3, "linear blit expected ~188, got {}", px.red());
    assert!((px.red() as i32 - 128).abs() > 20, "must not be sRGB 128");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p starbreaker-ui --lib blit_atlas_tinted_composites_in_linear_light 2>&1 | tail -20`
Expected: FAIL — current build-loop premultiplies in sRGB and `draw_pixmap` blends in sRGB → ≈128.

- [ ] **Step 3: Rewrite the blit as a linear texel loop**

Replace the body of `blit_atlas_image_tinted_with_mode` (the `premul` build loop + `Pixmap::from_vec` + `draw_pixmap`) with a direct linear composite. Add `u8_to_linear`, `blend_premul_linear`, `blend_premul_add_linear` to the `use crate::colour::{...}` line.

```rust
fn blit_atlas_image_tinted_with_mode(
    pixmap: &mut Pixmap,
    img: &RgbaImage,
    dx: i32,
    dy: i32,
    tint: [f32; 4],
    alpha: f32,
    blend_mode: BlendMode,
) {
    let tint_lin = [
        srgb_channel_to_linear(tint[0].clamp(0.0, 1.0)),
        srgb_channel_to_linear(tint[1].clamp(0.0, 1.0)),
        srgb_channel_to_linear(tint[2].clamp(0.0, 1.0)),
    ];
    let opacity = alpha.clamp(0.0, 1.0);
    let pw = pixmap.width() as i32;
    let ph = pixmap.height() as i32;
    let dw = pixmap.width();
    let dd = pixmap.data_mut();
    for (sy, row) in img.rows().enumerate() {
        let py = dy + sy as i32;
        if py < 0 || py >= ph {
            continue;
        }
        for (sx, texel) in row.enumerate() {
            let px = dx + sx as i32;
            if px < 0 || px >= pw {
                continue;
            }
            // Texture RGB linearised via LUT, modulated by tint in linear; alpha
            // = texel alpha × tint alpha × node opacity. Build a premultiplied
            // LINEAR-sRGB source byte, then reuse the shared blend primitive.
            let a = (texel.0[3] as f32 / 255.0) * tint[3] * opacity;
            if a <= 0.0 && blend_mode != BlendMode::Plus {
                continue;
            }
            let src = [
                (linear_channel_to_srgb((u8_to_linear(texel.0[0]) * tint_lin[0]).clamp(0.0, 1.0)) * a * 255.0)
                    .round().clamp(0.0, 255.0) as u8,
                (linear_channel_to_srgb((u8_to_linear(texel.0[1]) * tint_lin[1]).clamp(0.0, 1.0)) * a * 255.0)
                    .round().clamp(0.0, 255.0) as u8,
                (linear_channel_to_srgb((u8_to_linear(texel.0[2]) * tint_lin[2]).clamp(0.0, 1.0)) * a * 255.0)
                    .round().clamp(0.0, 255.0) as u8,
                (a * 255.0).round().clamp(0.0, 255.0) as u8,
            ];
            let di = ((py * dw as i32 + px) * 4) as usize;
            let mut d = [dd[di], dd[di + 1], dd[di + 2], dd[di + 3]];
            match blend_mode {
                BlendMode::Plus => crate::colour::blend_premul_add_linear(&mut d, src),
                _ => crate::colour::blend_premul_linear(&mut d, src),
            }
            dd[di..di + 4].copy_from_slice(&d);
        }
    }
}
```

(The `src` bytes are premultiplied LINEAR-modulated sRGB: the tint modulation is done in linear, then re-encoded to sRGB so `blend_premul_linear`'s unpremultiply→linear round-trips it back correctly. For a white texel this collapses to exactly the carve-out's `tint_lin[c] * coverage`, which is what Task 6's fold depends on.)

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p starbreaker-ui --lib blit_atlas_tinted_composites_in_linear_light 2>&1 | tail -20`
Expected: PASS.

- [ ] **Step 5: Confirm colour-only diff, TDD-tier check, commit**

Run: `git diff --stat` (only `engine_01.rs` blit changed).
Run: `bash scripts/ui_check.sh 2>&1 | tail -5`
Expected: `ui_check: ALL GREEN`.

```bash
git add crates/starbreaker-ui/src/ir_compose/engine_01.rs
git commit -m "ui(B4): tinted texture blits composite in linear light (texture sRGB ingest folded in)"
```

---

## Task 4: Manual clip composites → linear

Linearise the two hand-written clip composites: `composite_clip_region_pixmap` (`engine_02.rs:173`, premultiplied integer source-over) and `composite_clip_region_image` (`engine_02.rs:228`, straight-alpha f32 source-over).

**Files:**
- Modify: `crates/starbreaker-ui/src/ir_compose/engine_02.rs:173-202` and `:228-256`
- Test: `crates/starbreaker-ui/src/ir_compose/engine_02.rs` `#[cfg(test)]`

**Interfaces:**
- Consumes: `crate::colour::{blend_premul_linear, blend_straight_linear}` (Task 1). Add to the file's imports.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn composite_clip_pixmap_blends_in_linear() {
    let mut dst = Pixmap::new(2, 2).unwrap();
    dst.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
    let mut src = Pixmap::new(2, 2).unwrap();
    src.fill(tiny_skia::Color::from_rgba8(128, 128, 128, 128)); // premul 50% white
    let clip = UiIrRect { x: 0.0, y: 0.0, w: 2.0, h: 2.0 };
    composite_clip_region_pixmap(&mut dst, &src, &clip);
    let px = dst.pixel(0, 0).unwrap();
    assert!((px.red() as i32 - 188).abs() <= 3, "linear clip pixmap expected ~188, got {}", px.red());
}

#[test]
fn composite_clip_image_blends_in_linear() {
    let mut dst = RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255]));
    let src = RgbaImage::from_pixel(2, 2, image::Rgba([255, 255, 255, 128])); // straight 50% white
    let clip = UiIrRect { x: 0.0, y: 0.0, w: 2.0, h: 2.0 };
    composite_clip_region_image(&mut dst, &src, &clip);
    let px = dst.get_pixel(0, 0);
    assert!((px[0] as i32 - 188).abs() <= 3, "linear clip image expected ~188, got {}", px[0]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p starbreaker-ui --lib composite_clip 2>&1 | tail -20`
Expected: FAIL — both currently blend in sRGB → ≈128.

- [ ] **Step 3: Linearise both loops**

`composite_clip_region_pixmap` — replace the inner `for c in 0..4` integer premul source-over (`:195-199`) with the shared premul-linear primitive:

```rust
for y in y0..y1 {
    let row = (y * w) as usize;
    for x in x0..x1 {
        let i = (row + x as usize) * 4;
        if src_data[i + 3] == 0 {
            continue;
        }
        let src = [src_data[i], src_data[i + 1], src_data[i + 2], src_data[i + 3]];
        let mut d = [dst_data[i], dst_data[i + 1], dst_data[i + 2], dst_data[i + 3]];
        crate::colour::blend_premul_linear(&mut d, src);
        dst_data[i..i + 4].copy_from_slice(&d);
    }
}
```

`composite_clip_region_image` — replace the `out_a` / `for c in 0..3` straight-alpha body (`:244-253`) with:

```rust
let sp = src.get_pixel(x as u32, y as u32);
let sa = sp[3] as f32 / 255.0;
if sa <= 0.0 {
    continue;
}
let dp = dst.get_pixel_mut(x as u32, y as u32);
crate::colour::blend_straight_linear(&mut dp.0, [sp[0], sp[1], sp[2]], sa);
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p starbreaker-ui --lib composite_clip 2>&1 | tail -20`
Expected: PASS — both.

- [ ] **Step 5: Confirm colour-only diff, TDD-tier check, commit**

Run: `bash scripts/ui_check.sh 2>&1 | tail -5` — Expected: `ui_check: ALL GREEN`.

```bash
git add crates/starbreaker-ui/src/ir_compose/engine_02.rs
git commit -m "ui(B4): manual clip-region composites (pixmap + image) blend in linear light"
```

---

## Task 5: Glyph rasterisers (ttf + swf) → linear (text antialiasing, NO carve-out)

Text antialiasing IS included — image-only linear is explicitly not engine-faithful (runbook :422-423). Route both per-glyph-pixel blends through the shared primitives.

**Files:**
- Modify: `crates/starbreaker-ui/src/text/ttf_draw.rs:106-113` (glyph coverage closure)
- Modify: `crates/starbreaker-ui/src/text/swf_draw.rs:376-405` (`blend_pixmap_onto_image`)
- Test: `crates/starbreaker-ui/src/text/` test module (or `swf_draw.rs` `#[cfg(test)]` — `blend_pixmap_onto_image` is directly testable)

**Interfaces:**
- Consumes: `crate::colour::{blend_premul_linear, blend_straight_linear}` (Task 1). Add `use crate::colour::...` to each file.
- The ttf closure change is a one-line delegation to `blend_straight_linear`, whose exact math is already covered by Task 1's `blend_straight_linear_white_over_black_is_linear_not_srgb`; the focused per-site test here targets the standalone `blend_pixmap_onto_image` (swf).

- [ ] **Step 1: Write the failing test (swf standalone)**

Add to `crates/starbreaker-ui/src/text/swf_draw.rs` `#[cfg(test)]`:

```rust
#[test]
fn blend_pixmap_onto_image_composites_in_linear() {
    let mut img = RgbaImage::from_pixel(2, 2, image::Rgba([0, 0, 0, 255]));
    let mut src = Pixmap::new(2, 2).unwrap();
    src.fill(tiny_skia::Color::from_rgba8(128, 128, 128, 128)); // premul 50% white
    let clip = Rect { x: 0.0, y: 0.0, w: 2.0, h: 2.0 };
    blend_pixmap_onto_image(&src, &mut img, clip);
    let px = img.get_pixel(0, 0);
    assert!((px[0] as i32 - 188).abs() <= 3, "linear swf glyph blend expected ~188, got {}", px[0]);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p starbreaker-ui --lib blend_pixmap_onto_image_composites_in_linear 2>&1 | tail -20`
Expected: FAIL — current premul source-over in sRGB → ≈128.

- [ ] **Step 3: Route both rasterisers through the shared primitives**

`text/swf_draw.rs` `blend_pixmap_onto_image` — replace the `dst[0..3]` / `dst[3]` body (`:397-402`) with:

```rust
let src_a = src.alpha();
if src_a == 0 {
    continue;
}
let dst = img.get_pixel_mut(px as u32, py as u32);
crate::colour::blend_premul_linear(&mut dst.0, [src.red(), src.green(), src.blue(), src_a]);
```

(`src` is a premultiplied `tiny_skia::PremultipliedColorU8`; `.red()/.green()/.blue()` are the premultiplied channels — exactly `blend_premul_linear`'s input contract.)

`text/ttf_draw.rs` glyph coverage closure — replace the `pixel[0..3]` mul_add lines + the `pixel[3]` update (`:108-113`) with:

```rust
let pixel = img.get_pixel_mut(px as u32, py as u32);
let src_a = coverage * colour[3] as f32 / 255.0;
crate::colour::blend_straight_linear(&mut pixel.0, [colour[0], colour[1], colour[2]], src_a);
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p starbreaker-ui --lib blend_pixmap_onto_image_composites_in_linear 2>&1 | tail -20`
Expected: PASS. Also run the existing text suite to confirm no glyph-layout regression: `cargo test -p starbreaker-ui --lib text:: 2>&1 | tail -20` — Expected: PASS (any test asserting a specific sRGB-blended glyph edge value must be updated to the linear value; note it in the commit if so).

- [ ] **Step 5: Confirm colour-only diff, TDD-tier check, commit**

Run: `bash scripts/ui_check.sh 2>&1 | tail -5` — Expected: `ui_check: ALL GREEN`.

```bash
git add crates/starbreaker-ui/src/text/ttf_draw.rs crates/starbreaker-ui/src/text/swf_draw.rs
git commit -m "ui(B4): ttf + swf glyph antialiasing blends in linear light"
```

---

## Task 6: Fold the white-mask carve-out into the general blit

The general blit is now linear (Task 3), so `blit_white_mask_overlay_linear` is redundant and, if left standalone, is a second linear implementation to keep in sync. Delete it; route white-mask nodes through the general blit with their existing tint selection. Keep the test's intent (spec) by retargeting it to the general blit and asserting the same `(68,38,8)`.

**Files:**
- Modify: `crates/starbreaker-ui/src/ir_compose/engine_01.rs:503-511` (the branch) and delete `:916-975` (`blit_white_mask_overlay_linear` + its doc comment)
- Modify: `crates/starbreaker-ui/src/ir_compose/engine_02.rs:1289-1320` (retarget `white_mask_overlay_composites_in_linear_light`)

**Interfaces:**
- After this task there is exactly ONE premultiplied linear blit path (`blit_atlas_image_tinted_with_mode`). `white_mask_overlay_tint` (`:882`) still selects the mask's Base tint; only the blit call changes.

- [ ] **Step 1: Retarget the existing test to the general blit (failing until the fold)**

In `crates/starbreaker-ui/src/ir_compose/engine_02.rs`, change the test body of `white_mask_overlay_composites_in_linear_light` to call the general blit with the white-mask tint (white texel a=146, node alpha 0.1, tint (1.0,0.62,0.22)):

```rust
#[test]
fn white_mask_overlay_composites_in_linear_light() {
    // The white-mask glow now routes through the GENERAL linear blit (the
    // dedicated carve-out was folded in — B4). White texel a=146/255, node
    // alpha 0.1, tint (1.0,0.62,0.22) over opaque black => sRGB ~(68,38,8).
    let mut pixmap = Pixmap::new(1, 1).expect("pixmap");
    pixmap.fill(tiny_skia::Color::from_rgba8(0, 0, 0, 255));
    let mut img = RgbaImage::new(1, 1);
    img.put_pixel(0, 0, image::Rgba([255, 255, 255, 146]));

    blit_atlas_image_tinted_with_mode(
        &mut pixmap,
        &img,
        0,
        0,
        [1.0, 0.6196, 0.2235, 1.0],
        0.1,
        BlendMode::SourceOver,
    );

    let px = pixmap.data();
    assert!(
        (px[0] as i32 - 68).abs() <= 2 && (px[1] as i32 - 38).abs() <= 2 && px[2] <= 10,
        "linear-light blend of the white mask glow, got ({}, {}, {}, {})",
        px[0], px[1], px[2], px[3]
    );
}
```

(`blit_atlas_image_tinted_with_mode` must be reachable from the test — it already lives in `engine_01` and the test module imports the compose items; if it is not currently `pub(crate)`, widen it to `pub(crate)` in Task 3's edit and note it. It is already reachable via the module glob.)

- [ ] **Step 2: Run the test to verify it still passes (Task 3 already made the general blit linear)**

Run: `cargo test -p starbreaker-ui --lib white_mask_overlay_composites_in_linear_light 2>&1 | tail -20`
Expected: PASS — Task 3's linear blit produces the same `(68,38,8)` for a white texel (the general path's per-texel `tint_lin × coverage` collapses to the old carve-out for a white texel). If it FAILS, STOP: the blit's tint-in-linear modulation diverges from the carve-out and must be reconciled before deleting the carve-out.

- [ ] **Step 3: Delete the carve-out and simplify the branch**

Delete `blit_white_mask_overlay_linear` and its doc comment (`engine_01.rs:916-975`). Replace the branch at `:503-511` with a single blit that only varies the tint + blend mode:

```rust
let tint = if fill_override.is_none() {
    match white_mask_overlay_tint(node, asset_ref, Some(&img), ctx) {
        Some(mask_tint) => mask_tint,
        None => image_tint_for_blit(node, asset_ref, fill_override, Some(&img), ctx),
    }
} else {
    image_tint_for_blit(node, asset_ref, fill_override, Some(&img), ctx)
};
let blend_mode = image_blend_mode_for_node(node, asset_ref);
blit_atlas_image_tinted_with_mode(pixmap, &img, draw_x, draw_y, tint, node.alpha, blend_mode);
```

(The white-mask path previously forced `SourceOver`; a white-mask node's `image_blend_mode_for_node` must resolve to `SourceOver` for this to be behaviour-preserving. Verify: white-mask masks are `.tif`/`.dds` with `render_shape` false → `image_blend_mode_for_node` returns `SourceOver` at `:1010`. Confirm the annunciator whole-image guard in Task 7 is a clean IMPROVEMENT/expected move, not a regression.)

- [ ] **Step 4: Run the test + confirm no dead code**

Run: `cargo test -p starbreaker-ui --lib white_mask 2>&1 | tail -20` — Expected: PASS.
Run: `cargo build -p starbreaker-ui 2>&1 | tail -5` — Expected: clean, no `dead_code` warning for a leftover carve-out.

- [ ] **Step 5: TDD-tier check, commit**

Run: `bash scripts/ui_check.sh 2>&1 | tail -5` — Expected: `ui_check: ALL GREEN`.

```bash
git add crates/starbreaker-ui/src/ir_compose/engine_01.rs crates/starbreaker-ui/src/ir_compose/engine_02.rs
git commit -m "ui(B4): fold white-mask carve-out into the now-linear general blit"
```

---

## Task 7: Re-freeze all 15 baselines (owner-gated) + close the interim prose debt

This is the capstone. Every whole-image sha256 in `tests/fixtures/ui_regression_freeze.json` moves (`"channels":"srgba"`); the IR-geometry snapshot must stay byte-identical. The owner PRE-APPROVED the re-freeze (2026-07-07), but the specific deltas are new information — surface the delta table as a checkpoint BEFORE running the freeze.

**Files:**
- Modify (by the freeze scripts): `crates/starbreaker-ui/tests/fixtures/ui_regression_freeze.json`
- Verify byte-identical: `crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_freeze.json`
- Modify (prose debt): `docs/ui-architecture-runbook.md:413-423`, `docs/ui-clipper-parity-handoff.md:278-282` (item 10) + §11, `docs/ui-process-improvements.md` (ledger 15 `:477-483`, ledger 18 `:508-513`, `:2442`, `:2456`), `docs/ui-reference.md` ("linear-light compositing gap" cells incl. `:255`).

**The 15 targets:**
- **7 platinum:** `ui_target_a`, `ui_target_b`, `eng_annunciator_master_left`, `eng_annunciator_master_right`, `clipper_target_master`, `clipper_g_force_ball_master`, `clipper_velocity_ball_master`.
- **8 gold:** `clipper_small_door`, `clipper_power_master`, `clipper_velocity_num_master`, `clipper_compass_master`, `clipper_self_master`, `clipper_master_mode_display_master`, `clipper_countermeasures_master`, `clipper_lrind_master`.

- [ ] **Step 1: Fresh release build + LOD0 export**

```bash
cargo build --release
SC_DATA_P4K="$SC_DATA_P4K" ./target/release/starbreaker entity export drak_clipper \
    "$HOME/projects/scorg_tools/ships" --kind decomposed --lod 0 --mip 0 --materials all
```

Expected: export completes; the UI PNGs land near the end of the run under `$HOME/projects/scorg_tools/ships/Data/UI/Generated/`.

- [ ] **Step 2: Confirm the IR-geometry snapshot is byte-identical (the only-colour-moved guard)**

```bash
bash scripts/validate_ui_snapshot_freeze.sh 2>&1 | tail -20
git status --porcelain crates/starbreaker-ui/tests/fixtures/ui_ir/ui_snapshot_freeze.json
```

Expected: validator GREEN and `git status` prints NOTHING for `ui_snapshot_freeze.json`. If the snapshot moved, STOP — the migration touched geometry (it must not); diagnose before proceeding. This is the load-bearing guard that only colour changed.

- [ ] **Step 3: Generate the whole-image + reference-distance delta table for ALL 15 targets**

For each target, capture (a) whether its whole-image sha256 changed vs the current freeze (expected: ALL 15 changed), and (b) its pixel distance to its in-game reference capture BEFORE (current committed render) vs AFTER (this export), using `scripts/ui_compare.py`. The render↔reference file mapping is in the per-screen dossier (`crates/starbreaker-ui/docs/ui-reference.md` §3). Template:

```bash
# per target: distance of the freshly-exported render to its reference capture
python3 scripts/ui_compare.py <exported-render.png> <reference-capture.png>
```

Build a Markdown table:

| target | tier | sha256 changed | ref-dist before | ref-dist after | verdict |
|---|---|---|---|---|---|
| eng_annunciator_master_left | platinum | yes | … | … | TOWARD (good) |
| … (all 15) | | | | | |

**Adjudication rule (per research §5 + evidence):** movement TOWARD the reference capture = expected/good; movement AWAY = STOP/regression (do not freeze — re-open and diagnose). Concrete evidence checkpoint on the annunciator chiclet edge: the reference reads ~(45,25,7) at the top and ~(71,48,15) at the side; the sRGB renderer produced ~(6,4,1)/(13,8,3); the predicted linear result is ~(39,20,3)/(68,38,8). After this migration the chiclet edge pixels must land near the linear prediction (i.e. much closer to the reference), not the sRGB values. Sample those pixels directly for `eng_annunciator_master_left/right` and record them in the table.

- [ ] **Step 4: OWNER CHECKPOINT — present the delta table before freezing**

Present the complete delta table (all 15 rows + the annunciator chiclet pixel samples) to the owner. The re-freeze is pre-approved, but these specific deltas are new information — the owner sees them BEFORE the baseline is overwritten. Do NOT run the freeze until the table is surfaced and every row is adjudicated TOWARD/expected. Any AWAY row is a stop-and-diagnose, never a freeze-to-pass.

- [ ] **Step 5: Run the freeze cycle (export is current → skip re-export)**

```bash
bash scripts/ui_freeze_cycle.sh --approver owner --reason "linear-light compositing migration" --skip-export
```

This rebuilds release, cleans stale `*-current.png`, freezes the image artifacts, and runs both validators + `ui_check.sh --full`. Expected: ends GREEN. (Use `--skip-export` only because Step 1's export is known-current; if anything rebuilt the exporter since, drop the flag.)

- [ ] **Step 6: Confirm both validators + `--full` GREEN**

```bash
bash scripts/validate_ui_regression_artifacts.sh 2>&1 | tail -10
bash scripts/validate_ui_snapshot_freeze.sh 2>&1 | tail -10
bash scripts/ui_check.sh --full 2>&1 | tail -10
```

Expected: all three GREEN (`ui_check: ALL GREEN`). The 15 whole-image guards now pass against the re-frozen baselines; the IR snapshot still validates byte-identical.

- [ ] **Step 7: Re-measure render/encode perf (per the baseline method)**

```bash
SB_UI_TIMING=1 RAYON_NUM_THREADS=1 ./target/release/starbreaker entity export drak_clipper \
    "$HOME/projects/scorg_tools/ships" --kind decomposed --lod 0 --mip 0 --materials all 2>&1 \
    | grep -i "render\|encode\|\[timing\]"
```

Expected: render + encode timers still a small fraction of wall (ir_compile dominates ~93%). Note the new render/encode figures in the ledger; the u8→linear LUT keeps the sRGB→linear direction off `powf` on the straight-alpha hot paths. If render regressed materially (unexpected given ~1% share), record it but it does not gate the freeze.

- [ ] **Step 8: Close the interim linear-light prose debt**

Update, in the same commit as the freeze:
- `docs/ui-architecture-runbook.md:413-423` — remove/close the gated open-debt entry; state B4 landed (renderer-wide linear, no carve-out) and record the achieved annunciator chiclet-edge values.
- `docs/ui-clipper-parity-handoff.md:278-282` (item 10) + §11 — mark item 10 resolved.
- `docs/ui-process-improvements.md` — append a ledger entry recording the arc (delta table summary, IR snapshot byte-identical, perf figures); close the ledger-15/18 forward pointers and the `:2442`/`:2456` "pointing at B4" notes.
- `docs/ui-reference.md` — grep for "linear-light compositing gap" and clear each status cell (incl. `clipper_master_mode_display_master` at `:255`) now that its target re-froze.

```bash
grep -rn "linear-light compositing gap" crates/starbreaker-ui/docs/ docs/
```

- [ ] **Step 9: Commit the re-freeze + docs**

```bash
git add crates/starbreaker-ui/tests/fixtures/ui_regression_freeze.json \
        crates/starbreaker-ui/docs/ui-architecture-runbook.md \
        crates/starbreaker-ui/docs/ui-clipper-parity-handoff.md \
        crates/starbreaker-ui/docs/ui-process-improvements.md \
        crates/starbreaker-ui/docs/ui-reference.md
git commit -m "ui(B4): re-freeze 15 gold/platinum baselines for linear-light compositing; close interim debt"
```

(Doc paths are under `crates/starbreaker-ui/docs/` per memory `ui-docs-location`; the `docs/…` line refs in this plan are shorthand for that location — confirm the actual path with `git status` before adding.)

---

## Self-Review

**1. Spec coverage:**
- STRATEGY A (per-site manual linear composite into u8-sRGB pixmap, generalising the carve-out) — Tasks 2–5 all use the scratch/loop + shared premul/straight primitives; storage + `pixmap_to_rgba_image` + `encode_png` untouched. ✔
- Promote the two channel helpers to a shared module + add the u8→linear LUT — Task 1 (`crate::colour`, `u8_to_linear` OnceLock LUT). ✔
- Fold `blit_white_mask_overlay_linear`, keep its test — Task 6 (delete + retarget test to general blit). ✔
- NO carve-outs incl. text AA (ttf + swf) — Task 5. Dead `compose/` twins ignored (not touched). ✔
- IR-geometry snapshot must NOT move — Task 7 Step 2 (byte-identical guard) + per-task "colour-only diff" checks. ✔
- Per-site grouping: flat/shape fills (T2), blit premul loop incl. texture sRGB ingest (T3), `composite_clip_region_*` (T4), ttf + swf (T5) — each a commit with a focused known-linear-vs-sRGB test (white-over-black → ~188 ≠ 128). ✔
- Additive (`Plus`) handled — `blend_premul_add_linear` (T1) wired in T2 (`fill_linear`) and T3 (blit). ✔
- Re-freeze protocol (fresh LOD0 export → whole-image + IR delta table for all 15 → per-identity adjudication vs references → owner delta-table checkpoint BEFORE freeze → `ui_freeze_cycle.sh --approver owner --reason "linear-light compositing migration"` → both validators + `--full` → close prose debt) — Task 7, Steps 1-9, with the owner checkpoint at Step 4 explicitly BEFORE the freeze at Step 5. ✔
- Intermediate `--full` RED expected / geometry-IR GREEN is the guard — stated in Global Constraints and per-task Step 5. ✔

**2. Placeholder scan:** No TBD/TODO/"handle edge cases"/"similar to Task N". Every code step shows real code; every command shows expected output. The one repeated pattern (`fill_linear` wrapping of the two `engine_01` direct draws + the polygon fill in T2 Step 3) is described with its exact bounds source rather than re-pasting the closure — acceptable as the closure body is shown fully for `fill_rect`/`fill_rounded_rect`.

**3. Type consistency:** `blend_straight_linear(dst: &mut [u8;4], src_rgb: [u8;3], src_a: f32)`, `blend_premul_linear(dst: &mut [u8;4], src: [u8;4])`, `blend_premul_add_linear(dst: &mut [u8;4], src: [u8;4])`, `u8_to_linear(v: u8) -> f32`, `fill_linear(dst, bounds: TskRect, blend_mode: BlendMode, draw: impl FnOnce(&mut Pixmap, Transform))` are used identically across Tasks 1-6. `blit_atlas_image_tinted_with_mode`'s signature is unchanged (Task 3) and reused verbatim in Task 6's test. `image::RgbaImage` pixels accessed as `.0` (`[u8;4]`) consistently in T4/T5.
