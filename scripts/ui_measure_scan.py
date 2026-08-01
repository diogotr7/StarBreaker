#!/usr/bin/env python3
"""Scan-style pixel measurement sub-commands for UI parity work.

Split out of ui_measure.py (which keeps the contamination-guarded box/IR
measurer and dispatches here). Implements the flat key=value measurements
agents were hand-rolling as inline PIL heredocs:

    profile     mean-intensity profile along an axis + indices above threshold
    bands       contiguous above-threshold runs (start,end,center,length CSV)
    mask-bbox   bbox/count/mean of a sandboxed colour-mask expression
    capheight   ink extent of text inside a horizontal band
    sidebyside  zoomed NEAREST A|B comparison strip
    selfcheck   synthetic-image regression for the band/run/mask logic

Entry points: `run_subcommand(argv)` (called by ui_measure.py) and direct
`uv run python scripts/ui_measure_scan.py <subcommand> ...`.
"""

import argparse
import ast
import sys

from PIL import Image



def _die(msg):
    sys.exit(f"error: {msg}")


def load_gray(path, invert=False):
    """Greyscale float array, or a clear error for a missing/unreadable file."""
    import numpy as np
    import os
    if not os.path.exists(path):
        _die(f"no such image: {path}")
    try:
        im = np.asarray(Image.open(path).convert("L")).astype(float)
    except Exception as exc:  # unreadable / not an image
        _die(f"cannot read image {path}: {exc}")
    return (255.0 - im) if invert else im


def load_rgb(path):
    import numpy as np
    import os
    if not os.path.exists(path):
        _die(f"no such image: {path}")
    try:
        arr = np.asarray(Image.open(path).convert("RGB")).astype(np.int32)
    except Exception as exc:
        _die(f"cannot read image {path}: {exc}")
    return arr


def parse_band(spec, extent, label="band"):
    """`a:b` -> half-open (a, b) pixel range. A `.` in the spec means fractions."""
    if spec is None:
        return 0, extent
    parts = spec.split(":")
    if len(parts) != 2 or not parts[0].strip() or not parts[1].strip():
        _die(f"{label} must be 'a:b' (px) or 'a.f:b.f' (fractions) — got {spec!r}")
    try:
        a, b = (float(p) for p in parts)
    except ValueError:
        _die(f"{label} bounds must be numbers — got {spec!r}")
    if "." in spec:
        if not (0.0 <= a <= 1.0 and 0.0 <= b <= 1.0):
            _die(f"{label} fractions must be within 0.0..1.0 — got {spec!r}")
        a, b = a * extent, b * extent
    a, b = int(round(a)), int(round(b))
    a, b = max(0, a), min(extent, b)
    if b <= a:
        _die(f"{label} {spec!r} is empty after clamping to 0..{extent}")
    return a, b


def axis_profile(im, axis, band=None, cross=None):
    """Mean intensity along `axis` ('row'|'col') over the band, cross-averaged.

    Returns (values, offset) — values[i] is the mean of scan line offset+i.
    """
    h, w = im.shape
    if axis == "row":
        a, b = parse_band(band, h)
        c0, c1 = parse_band(cross, w, "cross")
        return im[a:b, c0:c1].mean(axis=1), a
    a, b = parse_band(band, w)
    r0, r1 = parse_band(cross, h, "cross")
    return im[r0:r1, a:b].mean(axis=0), a


def threshold_of(values, thr_frac):
    """base = median, thr = base + (max - base) * thr_frac."""
    import numpy as np
    base = float(np.median(values))
    peak = float(values.max())
    return base, peak, base + (peak - base) * thr_frac


def runs_of(indices, min_run):
    """Contiguous runs (start, end_inclusive) of at least min_run entries."""
    out = []
    start = prev = None
    for i in indices:
        if start is None:
            start = i
        elif i != prev + 1:
            if prev - start + 1 >= min_run:
                out.append((start, prev))
            start = i
        prev = i
    if start is not None and prev - start + 1 >= min_run:
        out.append((start, prev))
    return out


def cmd_profile(args):
    im = load_gray(args.image, args.invert)
    values, offset = axis_profile(im, args.axis, args.band, args.cross)
    base, peak, thr = threshold_of(values, args.thr_frac)
    hits = [offset + i for i, v in enumerate(values) if v > thr]
    print(f"image={args.image}")
    print(f"axis={args.axis}")
    print(f"size={im.shape[1]}x{im.shape[0]}")
    print(f"range={offset}:{offset + len(values)}")
    print(f"base={base:.3f}")
    print(f"max={peak:.3f}")
    print(f"thr_frac={args.thr_frac}")
    print(f"threshold={thr:.3f}")
    print(f"count={len(hits)}")
    print(f"first={hits[0] if hits else 'none'}")
    print(f"last={hits[-1] if hits else 'none'}")
    print("indices=" + ",".join(str(i) for i in hits))


def cmd_bands(args):
    im = load_gray(args.image, args.invert)
    values, offset = axis_profile(im, args.axis, args.band, args.cross)
    base, peak, thr = threshold_of(values, args.thr_frac)
    hits = [offset + i for i, v in enumerate(values) if v > thr]
    bands = runs_of(hits, args.min_run)
    print(f"image={args.image}")
    print(f"axis={args.axis}")
    print(f"size={im.shape[1]}x{im.shape[0]}")
    print(f"threshold={thr:.3f}")
    print(f"base={base:.3f}")
    print(f"max={peak:.3f}")
    print(f"min_run={args.min_run}")
    print(f"bands={len(bands)}")
    print("idx,start,end,center,length")
    for n, (s, e) in enumerate(bands):
        print(f"{n},{s},{e},{(s + e) / 2:g},{e - s + 1}")


# Expression sandbox for `mask-bbox`. Names are limited to R/G/B/np, and the
# node whitelist keeps eval() to arithmetic + comparisons: no attribute access
# outside np.*, no subscripts, no lambdas, no imports.
_MASK_NAMES = {"R", "G", "B", "np"}
_MASK_NODES = (
    ast.Expression, ast.BoolOp, ast.BinOp, ast.UnaryOp, ast.Compare,
    ast.Name, ast.Constant, ast.Load, ast.Attribute, ast.Call,
    ast.operator, ast.unaryop, ast.boolop, ast.cmpop,
)


class _MaskRewrite(ast.NodeTransformer):
    """`and`/`or`/`not` and chained comparisons -> numpy elementwise forms."""

    def visit_BoolOp(self, node):
        self.generic_visit(node)
        op = ast.BitAnd() if isinstance(node.op, ast.And) else ast.BitOr()
        expr = node.values[0]
        for nxt in node.values[1:]:
            expr = ast.BinOp(left=expr, op=op, right=nxt)
        return expr

    def visit_UnaryOp(self, node):
        self.generic_visit(node)
        if isinstance(node.op, ast.Not):
            return ast.UnaryOp(op=ast.Invert(), operand=node.operand)
        return node

    def visit_Compare(self, node):
        self.generic_visit(node)
        if len(node.ops) < 2:
            return node
        expr = None
        left = node.left
        for op, right in zip(node.ops, node.comparators):
            cmp_node = ast.Compare(left=left, ops=[op], comparators=[right])
            expr = cmp_node if expr is None else ast.BinOp(
                left=expr, op=ast.BitAnd(), right=cmp_node)
            left = right
        return expr


def eval_mask(expr, rgb):
    """Evaluate a colour-mask expression over R/G/B numpy planes."""
    import numpy as np
    try:
        tree = ast.parse(expr, mode="eval")
    except SyntaxError as exc:
        _die(f"bad --expr {expr!r}: {exc}")
    for node in ast.walk(tree):
        if not isinstance(node, _MASK_NODES):
            _die(f"--expr may not use {type(node).__name__} — allowed: "
                 "R/G/B/np, numbers, arithmetic, comparisons, and/or/not")
        if isinstance(node, ast.Name) and node.id not in _MASK_NAMES:
            _die(f"--expr may only reference R, G, B, np — got {node.id!r}")
    tree = ast.fix_missing_locations(_MaskRewrite().visit(tree))
    env = {"R": rgb[:, :, 0], "G": rgb[:, :, 1], "B": rgb[:, :, 2], "np": np}
    # eval() is deliberate and guarded: the AST above is whitelisted to
    # arithmetic/comparison nodes over the names R/G/B/np only, and builtins are
    # stripped, so nothing callable is reachable except numpy attributes.
    # ast.literal_eval cannot express `G > R + 30`, which is the whole point.
    try:
        mask = eval(compile(tree, "<expr>", "eval"), {"__builtins__": {}}, env)
    except Exception as exc:
        _die(f"--expr {expr!r} failed: {exc}")
    mask = np.asarray(mask)
    if mask.dtype != bool or mask.shape != rgb.shape[:2]:
        _die(f"--expr {expr!r} must yield a boolean mask the size of the image "
             f"— got {mask.dtype} {mask.shape}")
    return mask


def cmd_mask_bbox(args):
    import numpy as np
    rgb = load_rgb(args.image)
    mask = eval_mask(args.expr, rgb)
    h, w = mask.shape
    ys, xs = np.where(mask)
    print(f"image={args.image}")
    print(f"expr={args.expr}")
    print(f"size={w}x{h}")
    print(f"pixels={len(xs)}")
    print(f"frac={len(xs) / (w * h):.6f}")
    if len(xs) == 0:
        print("bbox=none")
        return
    x0, y0, x1, y1 = int(xs.min()), int(ys.min()), int(xs.max()), int(ys.max())
    print(f"bbox={x0},{y0},{x1},{y1}")
    print(f"x0={x0}")
    print(f"y0={y0}")
    print(f"x1={x1}")
    print(f"y1={y1}")
    print(f"w={x1 - x0 + 1}")
    print(f"h={y1 - y0 + 1}")
    print(f"centre={(x0 + x1) / 2:g},{(y0 + y1) / 2:g}")
    mean = rgb[mask].mean(axis=0)
    print(f"mean_rgb={mean[0]:.2f},{mean[1]:.2f},{mean[2]:.2f}")


def cmd_capheight(args):
    im = load_gray(args.image, args.invert)
    h = im.shape[0]
    values, offset = axis_profile(im, "row", args.band, args.cross)
    base, peak, thr = threshold_of(values, args.thr_frac)
    ink = [offset + i for i, v in enumerate(values) if v > thr]
    print(f"image={args.image}")
    print(f"size={im.shape[1]}x{h}")
    print(f"band={offset}:{offset + len(values)}")
    print(f"base={base:.3f}")
    print(f"max={peak:.3f}")
    print(f"threshold={thr:.3f}")
    print(f"ink_rows={len(ink)}")
    if not ink:
        print("first_ink=none")
        print("last_ink=none")
        print("cap_height_px=0")
        return
    print(f"first_ink={ink[0]}")
    print(f"last_ink={ink[-1]}")
    cap = ink[-1] - ink[0] + 1
    print(f"cap_height_px={cap}")
    print(f"cap_height_pct_of_h={100 * cap / h:.3f}")


def parse_crop(text, size):
    parts = text.split(",")
    if len(parts) != 4:
        _die(f"--crop expects x0,y0,x1,y1 — got {text!r}")
    try:
        x0, y0, x1, y1 = (int(p) for p in parts)
    except ValueError:
        _die(f"--crop bounds must be integers — got {text!r}")
    if x1 <= x0 or y1 <= y0:
        _die(f"--crop {text!r} is empty")
    w, h = size
    if x0 < 0 or y0 < 0 or x1 > w or y1 > h:
        _die(f"--crop {text!r} outside image bounds {w}x{h}")
    return x0, y0, x1, y1


def cmd_sidebyside(args):
    import os
    for path in (args.a, args.b):
        if not os.path.exists(path):
            _die(f"no such image: {path}")
    if args.zoom < 1:
        _die(f"--zoom must be >= 1 — got {args.zoom}")
    tiles = []
    for path in (args.a, args.b):
        img = Image.open(path).convert("RGB")
        if args.crop:
            img = img.crop(parse_crop(args.crop, img.size))
        img = img.resize((img.width * args.zoom, img.height * args.zoom),
                         Image.NEAREST)
        tiles.append(img)
    out_w = sum(t.width for t in tiles) + args.gap
    out_h = max(t.height for t in tiles)
    canvas = Image.new("RGB", (out_w, out_h))
    x = 0
    for tile in tiles:
        canvas.paste(tile, (x, 0))
        x += tile.width + args.gap
    canvas.save(args.out)
    print(f"a={args.a}")
    print(f"b={args.b}")
    print(f"crop={args.crop or 'full'}")
    print(f"zoom={args.zoom}")
    print(f"tile_a={tiles[0].width}x{tiles[0].height}")
    print(f"tile_b={tiles[1].width}x{tiles[1].height}")
    print(f"out={args.out}")
    print(f"out_size={out_w}x{out_h}")


def add_scan_args(sub, with_axis=True):
    sub.add_argument("--image", required=True, help="PNG to measure")
    if with_axis:
        sub.add_argument("--axis", required=True, choices=("row", "col"),
                         help="row = per-row profile, col = per-column profile")
    sub.add_argument("--band", help="a:b slice ALONG the scan axis "
                                    "(px, or fractions when the spec has a '.')")
    sub.add_argument("--cross", help="a:b slice of the PERPENDICULAR extent "
                                     "that gets averaged (same syntax)")
    sub.add_argument("--thr-frac", type=float, default=0.35,
                     help="threshold = median + (max - median) * thr_frac")
    sub.add_argument("--invert", action="store_true",
                     help="dark ink on a light ground")


def cmd_selfcheck(_args):
    """Synthetic-image regression for the band/run/mask logic (no fixtures)."""
    import tempfile, os
    img = Image.new("RGB", (200, 100), (0, 0, 0))
    px = img.load()
    for y in list(range(20, 30)) + list(range(60, 65)):   # two bright rows bands
        for x in range(50, 150):
            px[x, y] = (255, 255, 255)
    for y in range(80, 90):                               # a single hue patch
        for x in range(10, 20):
            px[x, y] = (50, 200, 60)
    with tempfile.TemporaryDirectory() as tmp:
        path = os.path.join(tmp, "synth.png")
        img.save(path)
        gray = load_gray(path)
        values, offset = axis_profile(gray, "row")
        _, _, thr = threshold_of(values, 0.35)
        hits = [offset + i for i, v in enumerate(values) if v > thr]
        assert runs_of(hits, 2) == [(20, 29), (60, 64)], runs_of(hits, 2)
        assert runs_of(hits, 6) == [(20, 29)], "min_run must drop short runs"
        band_hits, band_off = axis_profile(gray, "row", band="0.0:0.5")
        assert band_off == 0 and len(band_hits) == 50, "fractional band"
        cols, off = axis_profile(gray, "col", band="40:160")
        assert off == 40 and len(cols) == 120, "px band on the col axis"
        rgb = load_rgb(path)
        import numpy as np
        patch = eval_mask("G>R+30", rgb)
        ys, xs = np.where(patch)
        assert (xs.min(), ys.min(), xs.max(), ys.max()) == (10, 80, 19, 89)
        both = eval_mask("G>190 and R>40", rgb)
        assert both.sum() == 10 * 100 + 5 * 100 + 100, both.sum()
        assert eval_mask("40 < G < 210", rgb).sum() == 100, "chained compare"
    print("selfcheck=ok")


SUBCOMMANDS = ("profile", "bands", "mask-bbox", "capheight", "sidebyside",
               "selfcheck")


def run_subcommand(argv):
    parser = argparse.ArgumentParser(
        prog="ui_measure.py", description="pixel measurement sub-commands")
    subs = parser.add_subparsers(dest="cmd", required=True)

    add_scan_args(subs.add_parser(
        "profile", help="mean-intensity profile + indices above threshold"))
    bands = subs.add_parser(
        "bands", help="contiguous above-threshold runs along an axis")
    add_scan_args(bands)
    bands.add_argument("--min-run", type=int, default=2,
                       help="discard runs shorter than this many lines")

    mask = subs.add_parser("mask-bbox", help="bbox + count of a colour mask")
    mask.add_argument("--image", required=True)
    mask.add_argument("--expr", required=True,
                      help="mask over R,G,B planes, e.g. 'G>190 and R>40' or 'G>R+30'")

    add_scan_args(subs.add_parser(
        "capheight", help="ink extent of text inside a horizontal band"),
        with_axis=False)

    sbs = subs.add_parser("sidebyside", help="zoomed A|B comparison strip")
    sbs.add_argument("--a", required=True)
    sbs.add_argument("--b", required=True)
    sbs.add_argument("--crop", help="x0,y0,x1,y1 applied to BOTH images")
    sbs.add_argument("--zoom", type=int, default=3, help="NEAREST upscale factor")
    sbs.add_argument("--gap", type=int, default=0, help="px between the tiles")
    sbs.add_argument("--out", required=True, help="output PNG path")

    subs.add_parser("selfcheck", help="assert the band/run/mask logic on a "
                                      "generated synthetic image")

    args = parser.parse_args(argv)
    {
        "profile": cmd_profile,
        "bands": cmd_bands,
        "mask-bbox": cmd_mask_bbox,
        "capheight": cmd_capheight,
        "sidebyside": cmd_sidebyside,
        "selfcheck": cmd_selfcheck,
    }[args.cmd](args)


if __name__ == "__main__":
    run_subcommand(sys.argv[1:])
