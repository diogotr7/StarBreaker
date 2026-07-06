#!/usr/bin/env python3
"""Region-by-region render-vs-reference comparison for UI parity review
(docs/ui-process-improvements.md item 2).

Usage:
    python3 scripts/ui_compare.py <render.png> <reference.png> \
        [--regions <preset>] [--out-dir <dir>]

The reference is auto-scaled to the render's width BEFORE cropping, so
mismatched capture resolutions (e.g. a 1959x1513 screenshot vs a 1600x1200
render) compare correctly. Region boxes are in the RENDER's pixel space.

Rectification (plan P1.3, ledger item 21): manual captures are skewed.
When `<reference>.corners.json` exists next to the reference (or `--rectify
<corners.json>` is passed), the reference is warped onto the render's
rectangle via the homography from the capture's four screen-corner pixel
coordinates `{"tl":[x,y],"tr":[x,y],"br":[x,y],"bl":[x,y]}` (pure-python
DLT 8x8 solve + numpy bilinear warp — no OpenCV) instead of width-scaled.
Pick the corners in GIMP: hover the screen bezel corners and read the
pointer coordinates from the status bar.

Writes <out-dir>/cmp_full.png plus one cmp_<region>.png per preset region
(render on top / left, reference below / right). Review phases use this
script exclusively — no ad-hoc crop snippets.

Presets live in REGION_PRESETS below; add one row per screen as it is
worked. List available presets with --regions list.
"""

import argparse
import json
import os
import sys

from PIL import Image

# Boxes are (left, top, right, bottom) in the render's pixel space
# (1600x1200 for the Clipper RTT screens).
REGION_PRESETS = {
    "power": {  # Screen_Left_Lower_RTT vs reference Screen_Left_Lower_RTT.png
        "emissions": (40, 0, 1560, 170),
        "columns": (430, 170, 1430, 1030),
        "scrollbar": (430, 1000, 1430, 1080),
        "output_card": (60, 170, 560, 620),
        "battery_card": (60, 600, 560, 1060),
        "footer": (0, 1060, 1600, 1200),
    },
    "target": {  # Screen_Right_Upper_RTT vs reference Screen_Right_Upper_RTT.png
        "status_band": (0, 0, 1600, 420),
        "chevrons": (150, 260, 1450, 700),
        "footer": (0, 950, 1600, 1200),
    },
    "annunciator": {  # Screen_Annunciator_L (1920x432) vs Screen_Annunciator_L.png
        "pwr": (15, 15, 370, 420),
        "wpn": (399, 15, 754, 420),
        "thr": (783, 15, 1138, 420),
        "shld": (1167, 15, 1522, 420),
        "cool": (1551, 15, 1906, 420),
    },
    "door": {  # i_door_small_drak (1920x1080) vs Door-closed.png
        "header": (0, 0, 1920, 330),
        "status": (0, 330, 1920, 860),
        "bottom": (0, 860, 1920, 1080),
    },
    "compass": {  # Screen_Central_Compass (1920x587) vs compass_master.png
        "labels": (0, 0, 1920, 250),
        "ticks": (0, 280, 1920, 587),
        "lubber": (880, 0, 1040, 587),
    },
}


def harness_error(msg):
    print(f"HARNESS ERROR: {msg}", file=sys.stderr)
    sys.exit(2)


def homography_from_corners(width, height, corners):
    """3x3 homography H mapping render-space (x,y,1) to capture-space, from
    the four capture-pixel screen corners. Direct Linear Transform: each
    correspondence (x,y)->(u,v) contributes two rows of the 8x8 system in
    the unknowns a..h of H = [[a,b,c],[d,e,f],[g,h,1]]."""
    import numpy as np

    src = [(0, 0), (width, 0), (width, height), (0, height)]
    try:
        dst = [corners[key] for key in ("tl", "tr", "br", "bl")]
    except KeyError as missing:
        harness_error(f"corners file missing key {missing} (need tl/tr/br/bl)")
    rows, rhs = [], []
    for (x, y), (u, v) in zip(src, dst):
        rows.append([x, y, 1, 0, 0, 0, -u * x, -u * y])
        rhs.append(u)
        rows.append([0, 0, 0, x, y, 1, -v * x, -v * y])
        rhs.append(v)
    p = np.linalg.solve(np.array(rows, dtype=float), np.array(rhs, dtype=float))
    return np.array([[p[0], p[1], p[2]], [p[3], p[4], p[5]], [p[6], p[7], 1.0]])


def warp_capture_to_rect(capture, width, height, h_matrix):
    """Bilinear-warp `capture` onto a width x height rectangle: output pixel
    (x,y) samples the capture at H @ (x,y,1) (clamped at the borders)."""
    import numpy as np

    src = np.asarray(capture, dtype=float)
    ys, xs = np.mgrid[0:height, 0:width]
    denom = h_matrix[2, 0] * xs + h_matrix[2, 1] * ys + h_matrix[2, 2]
    u = (h_matrix[0, 0] * xs + h_matrix[0, 1] * ys + h_matrix[0, 2]) / denom
    v = (h_matrix[1, 0] * xs + h_matrix[1, 1] * ys + h_matrix[1, 2]) / denom
    u = np.clip(u, 0, src.shape[1] - 1.001)
    v = np.clip(v, 0, src.shape[0] - 1.001)
    u0 = np.floor(u).astype(int)
    v0 = np.floor(v).astype(int)
    fu = (u - u0)[..., None]
    fv = (v - v0)[..., None]
    out = (
        src[v0, u0] * (1 - fu) * (1 - fv)
        + src[v0, u0 + 1] * fu * (1 - fv)
        + src[v0 + 1, u0] * (1 - fu) * fv
        + src[v0 + 1, u0 + 1] * fu * fv
    )
    return Image.fromarray(out.round().astype("uint8"))


def rectify_reference(ref, render_width, render_height, corners_path):
    import json

    with open(corners_path) as handle:
        corners = json.load(handle)
    h_matrix = homography_from_corners(render_width, render_height, corners)
    warped = warp_capture_to_rect(ref, render_width, render_height, h_matrix)
    print(f"rectified via {corners_path}")
    return warped


def stack(a, b, gap=8, bg=(15, 15, 15)):
    """Side-by-side for tall crops, stacked for wide crops."""
    if a.width >= a.height:
        canvas = Image.new("RGB", (max(a.width, b.width), a.height + b.height + gap), bg)
        canvas.paste(a, (0, 0))
        canvas.paste(b, (0, a.height + gap))
    else:
        canvas = Image.new("RGB", (a.width + b.width + gap, max(a.height, b.height)), bg)
        canvas.paste(a, (0, 0))
        canvas.paste(b, (a.width + gap, 0))
    return canvas


def region_stats(name, render_crop, ref_crop):
    """Photometric comparison of a region: bright/dark pixel means and
    R-normalised ratios for both images.

    This is the method that diagnosed the linear-light compositing gap and
    identified the MissionObjectives icon slot: hue lives in the normalised
    ratios (capture casts attenuate G/B roughly uniformly per capture;
    bloom near bright elements lifts B). Judge HUE from ratios, never raw
    values, and estimate the capture's cast from a known anchor (footer
    text = Base, pip slabs = Bright) on the SAME reference before judging
    an unknown colour.
    """
    import numpy as np

    def stats(img):
        a = np.asarray(img, dtype=float)
        out = {}
        for label, mask in (
            ("bright", a.max(axis=2) > 110),
            ("dark", a.max(axis=2) < 60),
        ):
            px = a[mask]
            if len(px) == 0:
                out[label] = None
                continue
            mean = px.mean(axis=0)
            ratio = mean / mean[0] if mean[0] > 1e-6 else mean
            out[label] = (mean.round(0), ratio.round(2), len(px))
        return out

    r, f = stats(render_crop), stats(ref_crop)
    print(f"[{name}]")
    for label in ("bright", "dark"):
        for side, s in (("render", r[label]), ("ref   ", f[label])):
            if s is None:
                print(f"  {label:>6} {side}: (none)")
            else:
                mean, ratio, n = s
                print(
                    f"  {label:>6} {side}: mean=({mean[0]:.0f},{mean[1]:.0f},{mean[2]:.0f})"
                    f" ratio=(1,{ratio[1]:.2f},{ratio[2]:.2f}) n={n}"
                )

    # JSON view of the SAME rounded arrays used above, so --json never perturbs
    # the console output (the assert in plan A3 Step 1).
    def pack(side):
        out = {}
        for label in ("bright", "dark"):
            s = side[label]
            out[label] = None if s is None else {
                "mean": [float(x) for x in s[0]],
                "ratio": [float(x) for x in s[1]],
                "n": int(s[2]),
            }
        return out

    return {"render": pack(r), "ref": pack(f)}


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("render")
    ap.add_argument("reference")
    ap.add_argument("--regions", default=None, help="preset name (or 'list')")
    ap.add_argument("--out-dir", default="/tmp/ui_compare")
    ap.add_argument(
        "--stats",
        action="store_true",
        help="print per-region bright/dark pixel means + R-normalised ratios "
        "for render and reference (the photometric review method)",
    )
    ap.add_argument(
        "--rectify",
        default=None,
        help="corners.json ({\"tl\":[x,y],\"tr\":..,\"br\":..,\"bl\":..} capture "
        "coords of the screen corners); default: <reference>.corners.json "
        "is used automatically when present",
    )
    ap.add_argument(
        "--box",
        action="append",
        default=None,
        metavar="x0,y0,x1,y1",
        help="ad-hoc region (repeatable): stacked render|reference crop of an "
        "arbitrary rectangle, no preset needed. Honours --rectify and --stats. "
        "Writes cmp_box_<i>.png.",
    )
    ap.add_argument(
        "--json",
        default=None,
        metavar="PATH",
        help="also dump the per-region stats (region, box, crop path, "
        "render/ref bright+dark means & ratios) as JSON to PATH. Implies the "
        "stats computation; does NOT change console output.",
    )
    args = ap.parse_args()
    want_stats = args.stats or args.json is not None
    json_regions = []

    if args.regions == "list":
        for name, regions in REGION_PRESETS.items():
            print(f"{name}: {', '.join(regions)}")
        return

    if not os.path.isfile(args.render):
        harness_error(f"render not found: {args.render}")
    if not os.path.isfile(args.reference):
        harness_error(f"reference not found: {args.reference}")

    render = Image.open(args.render).convert("RGB")
    ref = Image.open(args.reference).convert("RGB")
    corners_path = args.rectify
    if corners_path is None and os.path.isfile(args.reference + ".corners.json"):
        corners_path = args.reference + ".corners.json"
    if corners_path is not None:
        if not os.path.isfile(corners_path):
            harness_error(f"corners file not found: {corners_path}")
        # Homography warp puts the reference exactly in the render's space.
        ref = rectify_reference(ref, render.width, render.height, corners_path)
    else:
        # Scale the reference into the render's pixel space.
        ref = ref.resize((render.width, round(ref.height * render.width / ref.width)))

    os.makedirs(args.out_dir, exist_ok=True)
    written = []

    full = stack(
        render.resize((render.width // 2, render.height // 2)),
        ref.resize((ref.width // 2, ref.height // 2)),
    )
    path = os.path.join(args.out_dir, "cmp_full.png")
    full.save(path)
    written.append(path)

    if args.regions:
        preset = REGION_PRESETS.get(args.regions)
        if preset is None:
            harness_error(f"unknown preset '{args.regions}' (try --regions list)")
        for name, box in preset.items():
            l, t, r, b = box
            r = min(r, render.width)
            a = render.crop((l, t, r, min(b, render.height)))
            bcrop = ref.crop((l, t, r, min(b, ref.height)))
            path = os.path.join(args.out_dir, f"cmp_{name}.png")
            stack(a, bcrop).save(path)
            written.append(path)
            if want_stats:
                data = region_stats(name, a, bcrop)
                json_regions.append({"region": name, "box": [l, t, r, min(b, render.height)], "crop": path, **data})
    elif want_stats and not args.box:
        data = region_stats("full", render, ref)
        json_regions.append({"region": "full", "box": [0, 0, render.width, render.height],
                             "crop": os.path.join(args.out_dir, "cmp_full.png"), **data})

    for index, spec in enumerate(args.box or []):
        try:
            l, t, r, b = (int(round(float(value))) for value in spec.split(","))
        except ValueError:
            harness_error(f"--box must be x0,y0,x1,y1 (got {spec!r})")
        a = render.crop((l, t, min(r, render.width), min(b, render.height)))
        bcrop = ref.crop((l, t, min(r, ref.width), min(b, ref.height)))
        path = os.path.join(args.out_dir, f"cmp_box_{index}.png")
        stack(a, bcrop).save(path)
        written.append(path)
        if want_stats:
            data = region_stats(f"box_{index}", a, bcrop)
            json_regions.append({"region": f"box_{index}",
                                 "box": [l, t, min(r, render.width), min(b, render.height)],
                                 "crop": path, **data})

    for p in written:
        print(p)

    if args.json is not None:
        with open(args.json, "w", encoding="utf-8") as fh:
            json.dump({"render": args.render, "reference": args.reference, "regions": json_regions}, fh, indent=2)
        print(f"wrote {args.json}", file=sys.stderr)  # stderr: stdout must stay identical


if __name__ == "__main__":
    main()
