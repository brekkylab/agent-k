"""contact_sheet.py — render a deck PDF into ONE labelled grid for visual QA.

The verify-B visual pass is the most token-expensive step: reading every
slide individually is N multimodal inputs. Instead build one contact sheet,
`read` it ONCE for the whole-deck overview (tofu, empty space, alignment,
off-canvas, chart colors), then deep-`read` only the slides that look suspect.

    python contact_sheet.py deck.pdf -o contact.png

Renders each PDF page with pypdfium2, writing a slide-N.png beside the PDF
(so "deep-read slide 7" maps to slide-7.png) and montaging them into one
sheet with each cell labelled by slide number.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


def render_pdf(pdf_path: str, dpi: int) -> list[str]:
    """Render each page to slide-N.png (1-based) beside the PDF; return paths."""
    import pypdfium2 as pdfium

    pdf = pdfium.PdfDocument(pdf_path)
    scale = dpi / 72.0
    out_dir = Path(pdf_path).parent
    paths = []
    try:
        for i in range(len(pdf)):
            img = pdf[i].render(scale=scale).to_pil()
            out = out_dir / f"slide-{i + 1}.png"
            img.save(out)
            paths.append(str(out))
    finally:
        pdf.close()
    return paths


def build(paths: list[str], out: str, cols: int, thumb_w: int, pad: int = 16) -> int:
    from PIL import Image, ImageDraw

    if not paths:
        print("contact_sheet: no pages to montage", file=sys.stderr)
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
    ap = argparse.ArgumentParser(description="Render a deck PDF into one labelled contact sheet.")
    ap.add_argument("pdf", help="the soffice-produced deck PDF")
    ap.add_argument("-o", "--out", default="contact.png", help="output PNG (default contact.png)")
    ap.add_argument("--cols", type=int, default=3, help="columns (default 3)")
    ap.add_argument("--thumb-w", type=int, default=560, help="per-slide thumbnail width px (default 560)")
    ap.add_argument("--dpi", type=int, default=75, help="PDF render resolution (default 75)")
    args = ap.parse_args(argv)
    return build(render_pdf(args.pdf, args.dpi), args.out, args.cols, args.thumb_w)


if __name__ == "__main__":
    raise SystemExit(main())
