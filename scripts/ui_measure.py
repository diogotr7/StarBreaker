#!/usr/bin/env python3
"""Contamination-guarded pixel measurer for UI parity adjudication
(plan P1.2, ledger items 19/21/23).

Measures glyph cap heights and colours inside an element box of a render or
reference capture, replacing the throwaway /tmp python of past arcs.

Usage:
    uv run python scripts/ui_measure.py <image.png> --box x0,y0,x1,y1 [options]
    uv run python scripts/ui_measure.py <image.png> --ir <ir.json> --node <id> [options]

Sub-commands (implemented in ui_measure_scan.py; plain key=value / CSV-ish
output, one fact per line — these replace the throwaway inline PIL heredocs
agents keep re-typing):

    uv run python scripts/ui_measure.py profile    --image P --axis row|col [...]
    uv run python scripts/ui_measure.py bands      --image P --axis row|col [...]
    uv run python scripts/ui_measure.py mask-bbox  --image P --expr 'G>190 and R>40'
    uv run python scripts/ui_measure.py capheight  --image P [--band 0.0:0.12]
    uv run python scripts/ui_measure.py sidebyside --a P1 --b P2 --out OUT.png

`--band a:b` selects a slice ALONG the scan axis (rows for --axis row, columns
for --axis col); `--cross a:b` restricts the perpendicular extent that is
averaged. Both accept pixels (`120:240`) or fractions of the extent — a spec
containing a `.` is read as fractions (`0.0:0.12`). Ranges are half-open
[a, b). Threshold is base + (max - base) * thr_frac with base = the profile
MEDIAN, so a mostly-empty band thresholds against its own background.

Options:
    --delta N              bright threshold = median luminance + N (default 30)
    --anchor x0,y0,x1,y1   also measure an anchor region (haze calibration)
    --anchor-rgb r,g,b     the anchor's TRUE colour (e.g. the palette Base
                           slot) — enables haze-corrected ratios

Output: one JSON object on stdout.

  glyph_runs   Bright-row runs inside the box: rows holding at least one
               pixel above the threshold, grouped into consecutive runs
               {y0, y1, h} (y1 exclusive, image coordinates). The CALLER
               picks the glyph run — the tool only flags suspects: a run is
               "suspect_contamination": true when its bright pixels touch
               BOTH the box's left and right edge columns (a bar/rule
               crossing the box — the footer-bar-line trap, ledger 19) or
               the run abuts the box's top/bottom row (likely truncated by
               the box). Single-edge contact is reported in "touches"
               without raising the flag (a glyph nudging one box edge).
  cap_height   Height of the tallest NON-suspect run (null if none).
  colour       Mean RGB over above-threshold pixels + R-normalised ratios +
               feature_width (horizontal px span of the bright pixels).
  warnings     Emitted (also to stderr) when feature_width <= 4: a thin bar/
               stroke/separator measured on a RECTIFIED capture has a smeared
               hue — measure its colour on the crisp ORIGINAL (ledger item 35).

Additive-haze photometric model (the refined form of docs/ui-reference.md
§3's method): captures carry a per-capture colour cast plus bloom that adds
a roughly constant offset to the channel ratios in a local region, so

    measured_ratio ≈ true_ratio + haze_offset      (ratio = G/R or B/R)

Measuring a region whose true colour IS known (the anchor: e.g. footer text
= brand Base, pip slabs = Bright) gives
haze_offset = anchor_measured_ratio − anchor_true_ratio, and the element's
corrected ratio is corrected = element_measured_ratio − haze_offset.
Judge hue from corrected ratios, never raw channel values.
"""

import argparse
import json
import sys

from PIL import Image

from ui_measure_scan import SUBCOMMANDS, run_subcommand


def parse_box(text):
    parts = [int(p) for p in text.split(",")]
    if len(parts) != 4:
        raise ValueError(f"expected x0,y0,x1,y1 — got {text!r}")
    x0, y0, x1, y1 = parts
    if x1 <= x0 or y1 <= y0:
        raise ValueError(f"empty box {text!r}")
    return x0, y0, x1, y1


def box_from_ir(ir_path, node_id):
    with open(ir_path) as handle:
        doc = json.load(handle)
    for node in doc.get("nodes", []):
        if node.get("id") == node_id:
            rect = node.get("computed_rect") or {}
            return (
                int(rect.get("x", 0)),
                int(rect.get("y", 0)),
                int(rect.get("x", 0) + rect.get("w", 0)),
                int(rect.get("y", 0) + rect.get("h", 0)),
            )
    sys.exit(f"error: node id {node_id} not in {ir_path}")


def luminance(px):
    return (px[0] + px[1] + px[2]) / 3.0


def measure_region(img, box, delta):
    """Bright mask + row runs + colour stats for one box."""
    x0, y0, x1, y1 = box
    width, height = img.size
    x0, y0 = max(0, x0), max(0, y0)
    x1, y1 = min(width, x1), min(height, y1)
    pixels = img.load()

    lums = sorted(
        luminance(pixels[x, y]) for y in range(y0, y1) for x in range(x0, x1)
    )
    median = lums[len(lums) // 2]
    threshold = median + delta

    bright_rows = {}  # y -> (has_left_edge, has_right_edge)
    bright_px = []
    min_bright_x = max_bright_x = None
    for y in range(y0, y1):
        edge_left = edge_right = False
        any_bright = False
        for x in range(x0, x1):
            px = pixels[x, y]
            if luminance(px) > threshold:
                any_bright = True
                bright_px.append(px)
                if min_bright_x is None or x < min_bright_x:
                    min_bright_x = x
                if max_bright_x is None or x > max_bright_x:
                    max_bright_x = x
                if x == x0:
                    edge_left = True
                if x == x1 - 1:
                    edge_right = True
        if any_bright:
            bright_rows[y] = (edge_left, edge_right)

    runs = []
    run_start = None
    prev = None
    for y in sorted(bright_rows):
        if run_start is None:
            run_start = y
        elif y != prev + 1:
            runs.append((run_start, prev + 1))
            run_start = y
        prev = y
    if run_start is not None:
        runs.append((run_start, prev + 1))

    glyph_runs = []
    for ry0, ry1 in runs:
        touches_left = any(bright_rows[y][0] for y in range(ry0, ry1))
        touches_right = any(bright_rows[y][1] for y in range(ry0, ry1))
        crossing_bar = touches_left and touches_right
        truncated = ry0 == y0 or ry1 == y1
        touches = [side for side, hit in (
            ("left", touches_left), ("right", touches_right),
            ("top", ry0 == y0), ("bottom", ry1 == y1),
        ) if hit]
        glyph_runs.append({
            "y0": ry0,
            "y1": ry1,
            "h": ry1 - ry0,
            "touches": touches,
            "suspect_contamination": crossing_bar or truncated,
        })

    colour = None
    if bright_px:
        n = len(bright_px)
        mean_r = sum(p[0] for p in bright_px) / n
        mean_g = sum(p[1] for p in bright_px) / n
        mean_b = sum(p[2] for p in bright_px) / n
        colour = {
            "mean_rgb": [round(mean_r, 2), round(mean_g, 2), round(mean_b, 2)],
            "ratios": ratios_of(mean_r, mean_g, mean_b),
            "pixels": n,
            "feature_width": (max_bright_x - min_bright_x + 1) if min_bright_x is not None else None,
        }

    clean = [r["h"] for r in glyph_runs if not r["suspect_contamination"]]
    # Vertical GAPS between consecutive bright runs. For an orange-on-orange
    # compass label the digit band and the (same-colour) tick band below it are
    # SEPARATE runs with a real gap between them — read the digit band's `h` and
    # the gap to the tick, rather than a single bbox that silently merges them
    # (a contaminated single bbox read "25% cap, clipping" when the digit was
    # 18.9% with a 27px tick gap; ledger 68).
    band_gaps = [
        glyph_runs[i + 1]["y0"] - glyph_runs[i]["y1"]
        for i in range(len(glyph_runs) - 1)
    ]
    return {
        "box": [x0, y0, x1, y1],
        "median_luminance": round(median, 2),
        "threshold": round(threshold, 2),
        "glyph_runs": glyph_runs,
        "band_gaps": band_gaps,
        "cap_height": max(clean) if clean else None,
        "colour": colour,
    }


def ratios_of(r, g, b):
    if r <= 0:
        return None
    return {"g_over_r": round(g / r, 4), "b_over_r": round(b / r, 4)}


def text_bands(image_path, thr_frac=0.62, min_row_px=3, gap=6):
    """Locate the bright text on a (text-only) screen and report its geometry.

    No box needed — finds the brightest pixels (> thr_frac of the image max
    luminance), then reports the text bbox + horizontal centre and the per-LINE
    cap-height bands as a PERCENT of image height. The percent is resolution-
    independent, so a render band can be compared directly with a reference band
    to expose a font-scale gap (the velocity-num HUD measured 1.9% vs the
    reference 20.6%/16.2% — ~7x too small).
    """
    import numpy as np
    im = np.asarray(Image.open(image_path).convert("L")).astype(float)
    h, w = im.shape
    thr = im.max() * thr_frac
    mask = im > thr
    ys, xs = np.where(mask)
    out = {"image": image_path, "size": [int(w), int(h)],
           "bright_threshold": round(float(thr), 1), "text_pixels": int(len(xs))}
    if len(xs) == 0:
        return out
    out["bbox"] = [int(xs.min()), int(ys.min()), int(xs.max()), int(ys.max())]
    out["centre_x"] = int((int(xs.min()) + int(xs.max())) // 2)
    out["centre_x_frac"] = round((int(xs.min()) + int(xs.max())) / 2 / w, 4)
    rowsum = mask.sum(1)
    rows = [y for y in range(h) if rowsum[y] > min_row_px]
    bands = []
    if rows:
        start = prev = rows[0]
        for y in rows[1:]:
            if y - prev > gap:
                bands.append((start, prev))
                start = y
            prev = y
        bands.append((start, prev))
    out["lines"] = [
        {"y0": int(a), "y1": int(b), "height_px": int(b - a + 1),
         "height_pct_of_h": round(100 * (b - a + 1) / h, 2)}
        for (a, b) in bands
    ]
    return out


def main():
    if len(sys.argv) > 1 and sys.argv[1] in SUBCOMMANDS:
        return run_subcommand(sys.argv[1:])

    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("image")
    parser.add_argument("--box", help="x0,y0,x1,y1 element box in image pixels")
    parser.add_argument("--ir", help="IR JSON (from ui render --dump-ir-dir)")
    parser.add_argument("--node", type=int, help="IR node id whose rect is the box")
    parser.add_argument("--delta", type=float, default=45.0,
                        help="bright threshold above the box's median luminance")
    parser.add_argument("--anchor", help="x0,y0,x1,y1 anchor region for haze calibration")
    parser.add_argument("--anchor-rgb", help="r,g,b TRUE colour of the anchor region")
    parser.add_argument("--text-bands", action="store_true",
                        help="text-screen mode: bright-text bbox + centre-x + per-line "
                             "band heights as %% of image height (no box needed)")
    parser.add_argument("--ref", help="reference image to compare --text-bands against")
    args = parser.parse_args()

    if args.text_bands:
        out = text_bands(args.image)
        if args.ref:
            ref = text_bands(args.ref)
            out["reference"] = ref
            rl, fl = out.get("lines"), ref.get("lines")
            if rl and fl:
                r_mean = sum(l["height_pct_of_h"] for l in rl) / len(rl)
                f_mean = sum(l["height_pct_of_h"] for l in fl) / len(fl)
                out["size_ratio_render_over_ref"] = round(r_mean / f_mean, 3) if f_mean else None
        json.dump(out, sys.stdout, indent=2)
        print()
        return

    if args.box:
        box = parse_box(args.box)
    elif args.ir and args.node is not None:
        box = box_from_ir(args.ir, args.node)
    else:
        parser.error("need --box or (--ir and --node)")

    img = Image.open(args.image).convert("RGB")
    result = {"image": args.image}
    result.update(measure_region(img, box, args.delta))

    # Thin-feature colour caveat (ledger item 35): a homography-RECTIFIED capture
    # interpolates a few-px-wide feature (a bar/stroke/dotted separator) with
    # whatever sits behind it, smearing its hue toward the background — a 2px
    # Accent1 bar measured ~Base on the rectified power reference and a real
    # colour bug was nearly closed as "faithful". Rectify for POSITION; measure
    # thin-feature COLOUR on the crisp ORIGINAL.
    feature_width = (result.get("colour") or {}).get("feature_width")
    if feature_width is not None and feature_width <= 4:
        warning = (
            f"bright feature is only {feature_width}px wide — if this image is a "
            "homography-rectified capture, its hue is smeared toward the background; "
            "measure COLOUR on the crisp ORIGINAL reference (rectify for position only)."
        )
        result["warnings"] = [warning]
        print(f"WARN: {warning}", file=sys.stderr)

    if args.anchor:
        anchor_box = parse_box(args.anchor)
        anchor = measure_region(img, anchor_box, args.delta)
        result["anchor"] = {
            "box": anchor["box"],
            "colour": anchor["colour"],
        }
        if args.anchor_rgb and anchor["colour"] and result["colour"]:
            r, g, b = (float(v) for v in args.anchor_rgb.split(","))
            true_ratios = ratios_of(r, g, b)
            measured = anchor["colour"]["ratios"]
            if true_ratios and measured:
                haze = {
                    "g_over_r": round(measured["g_over_r"] - true_ratios["g_over_r"], 4),
                    "b_over_r": round(measured["b_over_r"] - true_ratios["b_over_r"], 4),
                }
                element = result["colour"]["ratios"]
                result["anchor"]["true_rgb"] = [r, g, b]
                result["anchor"]["haze_offset"] = haze
                result["corrected_ratios"] = {
                    "g_over_r": round(element["g_over_r"] - haze["g_over_r"], 4),
                    "b_over_r": round(element["b_over_r"] - haze["b_over_r"], 4),
                }

    json.dump(result, sys.stdout, indent=2)
    print()


if __name__ == "__main__":
    main()
