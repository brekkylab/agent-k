"""contact_sheet.py — montage slide PNGs into ONE labelled grid image.

The verify-B visual pass is the most token-expensive step: reading every
slide PNG individually is N multimodal inputs. Instead, build one contact
sheet, `read` it ONCE for the whole-deck overview (tofu, empty space,
alignment, off-canvas, chart colors), then deep-`read` only the slides that
look suspect. Cuts vision inputs ~N→1 for the overview pass.

Usage (Pillow ships with python-pptx, so no extra install):
    python contact_sheet.py slide-1.png slide-2.png ... -o contact.png
    python contact_sheet.py --glob "slide*.png" -o contact.png --cols 3

Each cell is labelled with its slide number so you can say "deep-read
slide 7" and map it back to `slide-7.png`.
"""

from __future__ import annotations

import argparse
import glob as globmod
import re
import sys
from pathlib import Path


def _natkey(p: str):
    """Sort slide-2.png before slide-10.png (natural numeric order)."""
    nums = re.findall(r"\d+", Path(p).name)
    return (int(nums[-1]) if nums else 0, p)


def build(paths: list[str], out: str, cols: int, thumb_w: int, pad: int = 16) -> int:
    from PIL import Image, ImageDraw

    paths = sorted(paths, key=_natkey)
    if not paths:
        print("contact_sheet: no input images", file=sys.stderr)
        return 2

    thumbs = []
    for p in paths:
        im = Image.open(p).convert("RGB")
        h = max(1, round(thumb_w * im.height / im.width))
        thumbs.append(im.resize((thumb_w, h), Image.LANCZOS))

    cell_w = thumb_w
    cell_h = max(t.height for t in thumbs)
    label_h = 26
    rows = (len(thumbs) + cols - 1) // cols
    W = pad + cols * (cell_w + pad)
    H = pad + rows * (cell_h + label_h + pad)

    sheet = Image.new("RGB", (W, H), (82, 86, 89))   # neutral gray gutter
    draw = ImageDraw.Draw(sheet)
    for i, t in enumerate(thumbs):
        r, c = divmod(i, cols)
        x = pad + c * (cell_w + pad)
        y = pad + r * (cell_h + label_h + pad)
        draw.text((x + 2, y), f"slide {i + 1}", fill=(255, 255, 255))
        sheet.paste(t, (x, y + label_h))

    Path(out).parent.mkdir(parents=True, exist_ok=True)
    sheet.save(out)
    print(f"wrote {out} — {len(thumbs)} slides, {cols} cols, {W}x{H}px")
    return 0


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Montage slide PNGs into one contact sheet.")
    ap.add_argument("images", nargs="*", help="slide PNG paths")
    ap.add_argument("--glob", default=None, help="glob pattern instead of explicit paths")
    ap.add_argument("-o", "--out", default="contact.png", help="output PNG (default contact.png)")
    ap.add_argument("--cols", type=int, default=3, help="columns (default 3)")
    ap.add_argument("--thumb-w", type=int, default=560, help="per-slide thumbnail width px (default 560)")
    args = ap.parse_args(argv)

    paths = list(args.images)
    if args.glob:
        paths += globmod.glob(args.glob)
    if not paths:
        ap.error("provide image paths or --glob")
    return build(paths, args.out, args.cols, args.thumb_w)


if __name__ == "__main__":
    raise SystemExit(main())
