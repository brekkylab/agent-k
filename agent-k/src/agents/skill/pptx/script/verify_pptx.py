"""verify_pptx — design-system compliance checks for a built .pptx.

Usage (inside the sandbox):
    import sys
    sys.path.insert(0, "/workspace/skills/pptx/script")
    from verify_pptx import verify

    issues = verify("/workspace/artifacts/deck.pptx")
    # issues is a dict with five keys (palette, fonts, sizes, overflow,
    # page_numbers); each value is a list of problems or a summary.
    # An empty list / "ok" value means the check passed.

Run all checks after building and re-fix any non-empty result before
shipping. The checks are heuristic — they catch the most-violated
rules from SKILL.md, not every nuance.
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from pptx import Presentation
from pptx.oxml.ns import qn


# --- palette gallery (mirrors SKILL.md) ---------------------------------------

GALLERY: dict[str, set[str]] = {
    "Corporate Slate": {
        "0F172A", "2563EB", "F59E0B", "F8FAFC", "FFFFFF",
        "64748B", "E2E8F0", "10B981", "EF4444",
    },
    "Midnight Keynote": {
        "0B1220", "818CF8", "22D3EE", "1F2937", "94A3B8",
        "34D399", "F87171",
    },
    "Warm Editorial": {
        "7C2D12", "EA580C", "FACC15", "FFF7ED", "FFFFFF",
        "9A3412", "FED7AA", "15803D", "B91C1C",
    },
    "Forest Research": {
        "1F2937", "047857", "D97706", "FFFFFF", "F9FAFB",
        "6B7280", "E5E7EB", "059669", "DC2626",
    },
    "Mono Editorial": {
        "111827", "FFFFFF", "6B7280", "E5E7EB", "059669", "DC2626",
    },
    "Playful Violet": {
        "4C1D95", "14B8A6", "F472B6", "FAF5FF", "FFFFFF",
        "6D28D9", "E9D5FF", "16A34A", "E11D48",
    },
    "Sand & Ink": {
        "1C1917", "57534E", "B45309", "FAFAF9", "FFFFFF",
        "78716C", "E7E5E4", "15803D", "B91C1C",
    },
    "Glacier": {
        "0E7490", "0891B2", "F97316", "F0F9FF", "FFFFFF",
        "475569", "BAE6FD", "16A34A", "DC2626",
    },
}

# Common neutrals that aren't a brand colour and shouldn't trip the
# "off-gallery" alarm: pure white, pure black, very light grays.
NEUTRAL_TOLERATED: set[str] = {"FFFFFF", "000000"}


# --- helpers ------------------------------------------------------------------

def _has_cjk(text: str) -> bool:
    for ch in text:
        cp = ord(ch)
        if (0xAC00 <= cp <= 0xD7AF        # Hangul Syllables
            or 0x3040 <= cp <= 0x30FF     # Hiragana / Katakana
            or 0x4E00 <= cp <= 0x9FFF     # CJK Unified
            or 0x3400 <= cp <= 0x4DBF     # CJK Ext A
            or 0xFF00 <= cp <= 0xFFEF):   # Halfwidth / Fullwidth
            return True
    return False


def _ea_typeface(run) -> str | None:
    rPr = run._r.find(qn("a:rPr"))
    if rPr is not None:
        ea = rPr.find(qn("a:ea"))
        if ea is not None and ea.get("typeface"):
            return ea.get("typeface")
    para = run._r.getparent()
    if para is not None:
        pPr = para.find(qn("a:pPr"))
        if pPr is not None:
            defRPr = pPr.find(qn("a:defRPr"))
            if defRPr is not None:
                ea = defRPr.find(qn("a:ea"))
                if ea is not None and ea.get("typeface"):
                    return ea.get("typeface")
    return None


def _iter_runs(slide):
    for shape in slide.shapes:
        if not shape.has_text_frame:
            continue
        for para in shape.text_frame.paragraphs:
            for run in para.runs:
                yield shape, para, run


def _collect_fills(prs) -> list[str]:
    """All solid fill hex colors across the deck, uppercased."""
    out: list[str] = []
    for slide in prs.slides:
        for shape in slide.shapes:
            try:
                if shape.fill.type == 1:  # solid
                    out.append(str(shape.fill.fore_color.rgb).upper())
            except (AttributeError, TypeError):
                # No fill / theme-based fill / placeholder without fill.
                pass
            if shape.has_text_frame:
                for para in shape.text_frame.paragraphs:
                    for run in para.runs:
                        try:
                            rgb = run.font.color.rgb
                            if rgb is not None:
                                out.append(str(rgb).upper())
                        except (AttributeError, TypeError):
                            # Run inherits color from theme — no rgb to read.
                            pass
    return out


# --- checks -------------------------------------------------------------------

def check_palette(prs) -> dict[str, Any]:
    """Detect which gallery palette is in use, and report off-gallery colors.

    Coverage is *frequency-weighted* — each fill occurrence counts. A
    30-slide deck with one stray hex on one slide stays near 1.0, instead
    of crashing to a low fraction the way a set-based score would.

    Returns {"matched": <name>|"custom"|"none", "coverage": float,
              "off_gallery": [hex, ...]}.
    A deck is "compliant" if coverage >= 0.85 against one gallery palette.
    Off-gallery colors are returned so the model can decide whether they're
    intentional accents or accidental drift.
    """
    from collections import Counter

    fills = [f for f in _collect_fills(prs) if f not in NEUTRAL_TOLERATED]
    if not fills:
        return {"matched": "none", "coverage": 0.0, "off_gallery": []}

    counts = Counter(fills)
    total = sum(counts.values())
    best_name = "custom"
    best_coverage = 0.0
    best_off: set[str] = set(counts)

    for name, palette in GALLERY.items():
        matched = sum(c for h, c in counts.items() if h in palette)
        coverage = matched / total if total else 0.0
        if coverage > best_coverage:
            best_coverage = coverage
            best_name = name
            best_off = {h for h in counts if h not in palette}

    if best_coverage < 0.5:
        return {
            "matched": "custom",
            "coverage": round(best_coverage, 2),
            "off_gallery": sorted(counts),
        }
    return {
        "matched": best_name,
        "coverage": round(best_coverage, 2),
        "off_gallery": sorted(best_off),
    }


def check_fonts(prs) -> list[tuple[int, str]]:
    """Find CJK-containing runs whose East-Asian typeface is unset.

    Returns list of (slide_index, snippet) tuples. Empty = compliant.
    """
    issues: list[tuple[int, str]] = []
    for i, slide in enumerate(prs.slides, 1):
        for _shape, _para, run in _iter_runs(slide):
            text = run.text
            if not _has_cjk(text):
                continue
            if _ea_typeface(run) is None:
                snippet = text.strip()[:40]
                issues.append((i, snippet))
                # one report per run is enough; continue
    return issues


def check_size_discipline(prs) -> list[tuple[int, list[int]]]:
    """Flag slides that mix too many tier / out-of-tier sizes.

    The scale tiers (rounded bounds, roughly ±30% around the canonical pt):
        Caption  ~11pt   (8-15)
        Body     ~18pt   (16-24)
        Title    ~32pt   (25-44)
        Display  ~54pt   (45-90)
    Multiple distinct point sizes inside the SAME tier (e.g. 18 and 22,
    both Body) collapse to one — they're not the failure mode the rule
    targets. Out-of-tier sizes (>90pt dramatic beat, or random sizes
    like 17 / 23) each count individually.

    Threshold: 5+ distinct (tier ∪ out-of-tier) classes per slide → flag.
    With only four tiers, tripping the threshold requires all four tiers
    plus at least one stray size — a clear discipline violation.

    Returns list of (slide_idx, sizes_found_on_slide) where the count is 5+.
    """

    def tier(pt: int) -> str | None:
        if pt <= 0:
            return None
        if 8 <= pt <= 15:
            return "caption"
        if 16 <= pt <= 24:
            return "body"
        if 25 <= pt <= 44:
            return "title"
        if 45 <= pt <= 90:
            return "display"
        return None  # outside scale → dramatic-beat or out-of-range

    issues: list[tuple[int, list[int]]] = []
    for i, slide in enumerate(prs.slides, 1):
        tiers_used: set[str] = set()
        out_of_tier: set[int] = set()
        all_sizes: set[int] = set()
        for _shape, _para, run in _iter_runs(slide):
            try:
                if run.font.size:
                    pt = round(run.font.size.pt)
                    t = tier(pt)
                    if t is not None:
                        tiers_used.add(t)
                        all_sizes.add(pt)
                    else:
                        out_of_tier.add(pt)
                        all_sizes.add(pt)
            except AttributeError:
                continue
        classes = len(tiers_used) + len(out_of_tier)
        if classes >= 5:
            issues.append((i, sorted(all_sizes)))
    return issues


def check_overflow(prs) -> list[tuple[int, str, str]]:
    """Estimate text clipping in fixed-size text boxes.

    A fixed-size text box overflows when the estimated rendered height
    exceeds the box's drawable height (box minus internal margins).
    Korean glyphs count as 2 ASCII widths.

    **Autosize boxes are skipped.** Predicting whether their `SHAPE_TO_FIT_TEXT`
    growth will overlap a neighbor requires geometric analysis we don't do
    here, and the previous heuristic produced too many false positives
    (most flagged grows had empty space below and never caused visible
    overlap). Visual PNG inspection (Step 3.B) catches the real overlaps.

    Returns list of (slide_idx, snippet, reason).
    """
    issues: list[tuple[int, str, str]] = []
    for i, slide in enumerate(prs.slides, 1):
        for shape in slide.shapes:
            if not shape.has_text_frame:
                continue
            try:
                w_in = shape.width / 914400 if shape.width else 0
                h_in = shape.height / 914400 if shape.height else 0
            except (AttributeError, TypeError):
                continue
            if w_in <= 0 or h_in <= 0:
                continue
            tf = shape.text_frame

            # Skip autosize boxes — they grow rather than clip; whether the
            # grow actually overlaps a neighbor is a geometry question we
            # leave to visual inspection.
            try:
                if tf.auto_size and "SHAPE_TO_FIT_TEXT" in str(tf.auto_size):
                    continue
            except AttributeError:
                pass

            # Subtract internal margins so chars-per-line / available height
            # reflect the *drawable* area. python-pptx margins are EMU;
            # default L/R is 0.1" each, T/B 0.05".
            try:
                ml = (tf.margin_left or 0) / 914400
                mr = (tf.margin_right or 0) / 914400
                mt = (tf.margin_top or 0) / 914400
                mb = (tf.margin_bottom or 0) / 914400
            except AttributeError:
                ml = mr = 0.1
                mt = mb = 0.05
            effective_w = max(0.1, w_in - ml - mr)
            effective_h = max(0.1, h_in - mt - mb)

            total_h = 0.0
            sample = ""
            for para in tf.paragraphs:
                run_texts = [r.text for r in para.runs if r.text]
                if not run_texts:
                    continue
                text = "".join(run_texts)
                font_pt = 14
                for r in para.runs:
                    try:
                        if r.font.size:
                            font_pt = max(font_pt, round(r.font.size.pt))
                    except AttributeError:
                        pass
                ls = 1.2
                try:
                    if para.line_spacing and isinstance(para.line_spacing, float):
                        ls = para.line_spacing
                except AttributeError:
                    pass
                units = sum(2 if ord(ch) > 0x7F else 1 for ch in text)
                chars_per_line = max(1, int(effective_w * 12 * 14 / font_pt))
                lines = max(1, -(-units // chars_per_line))
                total_h += lines * font_pt * ls / 72
                if not sample:
                    sample = text.strip()[:40]

            grow_abs = total_h - effective_h
            grow_ratio = total_h / effective_h if effective_h > 0 else 1
            if grow_abs > 0.1 or grow_ratio > 1.25:
                issues.append((
                    i, sample,
                    f"text needs {total_h:.2f}\" but box is {effective_h:.2f}\""
                ))
    return issues


def check_page_numbers(prs) -> list[tuple[int, str]]:
    """Title / divider / closing slides must not carry a page number.

    Heuristic for divider: full-bleed Primary background (slide that's almost
    entirely one dark colored shape). Title = slide 1. Closing = last slide.

    Returns list of (slide_idx, reason).
    """
    import re

    n = len(prs.slides)
    if n == 0:
        return []

    # Page-number conventions on a bookend slide:
    #   "3 / 10", "3/10", "3·10"    → n[/·]N (1-3 digits each)
    #   "Page 3", "Page 03"          → "Page n" prefix (case-insensitive)
    #   "3 of 10", "3 / 10 pages"   → "n of N" form
    # Plain single numbers ("3") are deliberately NOT matched — too many
    # data-driven slides have a stray number in a caption-sized box.
    _PAGE_NUM_PATTERNS = (
        re.compile(r"\d{1,3}\s*[/·]\s*\d{1,3}"),
        re.compile(r"(?i)page\s+\d{1,3}"),
        re.compile(r"\d{1,3}\s+of\s+\d{1,3}"),
    )

    def looks_like_page_num(text: str) -> bool:
        t = text.strip()
        return any(p.fullmatch(t) for p in _PAGE_NUM_PATTERNS)

    def has_page_num(slide) -> bool:
        for _shape, _para, run in _iter_runs(slide):
            if looks_like_page_num(run.text):
                return True
        return False

    issues: list[tuple[int, str]] = []
    if has_page_num(prs.slides[0]):
        issues.append((1, "title slide carries a page number"))
    if n > 1 and has_page_num(prs.slides[-1]):
        issues.append((n, "closing slide carries a page number"))
    # Section dividers: heuristic = slide has < 4 text runs and a large
    # dark filled rect covering most of the slide. We skip the heuristic
    # here and leave divider detection to the model.
    return issues


def check_overlap(prs) -> list[tuple[int, str, str, float]]:
    """Flag visually colliding shape pairs on each slide.

    Catches three failure modes the per-box `overflow` check can't see:
      - a Picture sitting on top of bullet / narrative text (e.g. a
        matplotlib chart inserted too tall, overrunning the insight box);
      - a Table whose rows extend down into the bullet area below;
      - two substantive textboxes whose bboxes overlap each other.

    Each (shape A, shape B) pair is reported once. A pair is skipped
    when one shape *fully contains* the other (intentional layered
    designs such as a caption on top of a card / image).

    Returns (slide_idx, shape_A_label, shape_B_preview, overlap_sq_in).
    """
    THRESH_SQ_IN = 0.2
    MIN_TEXT_LEN = 10
    PICTURE_TYPE = 13       # MSO_SHAPE_TYPE.PICTURE
    GRAPHIC_FRAME = 19      # MSO_SHAPE_TYPE.GRAPHIC_FRAME (tables, charts)
    AUTO_SHAPE = 1          # MSO_SHAPE_TYPE.AUTO_SHAPE (chevrons, rounded rects, etc.)
    # An AutoShape is "decoration-only" — and skipped — when it's thin
    # in either dimension (signature marks, edge strips, hairlines,
    # corner blocks). Threshold: both edges ≥ 0.3" AND area ≥ 0.5 sq".
    AUTO_SHAPE_MIN_EDGE = 0.3
    AUTO_SHAPE_MIN_AREA = 0.5

    def bbox(sh):
        if sh.left is None or sh.top is None or sh.width is None or sh.height is None:
            return None
        return (
            sh.left / 914400, sh.top / 914400,
            sh.width / 914400, sh.height / 914400,
        )

    def overlap_area(a, b):
        al, at, aw, ah = a
        bl, bt, bw, bh = b
        ix = min(al + aw, bl + bw) - max(al, bl)
        iy = min(at + ah, bt + bh) - max(at, bt)
        if ix <= 0 or iy <= 0:
            return 0.0
        return ix * iy

    def mostly_contains(outer, inner, ratio=0.7):
        """True when `inner` is ≥ `ratio` of its area inside `outer`.

        Catches the intentional "text-inside-card" / "text-on-image"
        layering even when the inner textbox bbox extends slightly past
        the outer container's edge (loose textbox sizing).
        """
        inner_area = inner[2] * inner[3]
        if inner_area <= 0:
            return False
        return overlap_area(outer, inner) / inner_area >= ratio

    def center_inside(small, big):
        """True if the center of `small` falls inside `big`'s bbox.

        For KPI-card style layering, the text's textbox is often loose
        (extends slightly past the card edge) but the *visible content*
        sits at the center — so a center-in-container check matches the
        designer's intent even when bboxes don't strictly nest.
        """
        sl, st, sw, sh = small
        cx, cy = sl + sw / 2, st + sh / 2
        bl, bt, bw, bh = big
        return bl <= cx <= bl + bw and bt <= cy <= bt + bh

    def text_top_inside(text_b, container):
        """The text's *visible* content sits near the top of its bbox
        (default vertical_anchor=TOP). When the text top-edge is inside
        the container, the layering is intentional (text-on-card) even
        if the textbox bbox extends past the container's edge.
        """
        tl, tt, tw, _ = text_b
        cx = tl + tw / 2
        bl, bt, bw, bh = container
        return bl <= cx <= bl + bw and bt <= tt <= bt + bh

    def is_vertical_stack(a, b):
        """True when two boxes look like adjacent stacked elements.

        The eyebrow + title pattern, or stacked card descriptions, has
        bboxes that geometrically intersect (one extends down into the
        next) yet the rendered text never collides — text sits at the
        top of each box. Heuristic: if the y-overlap is < 30% of the
        shorter box's height AND the horizontal overlap is the whole
        narrower box, treat the pair as stacked.
        """
        al, at, aw, ah = a
        bl, bt, bw, bh = b
        y_overlap = max(0.0, min(at + ah, bt + bh) - max(at, bt))
        x_overlap = max(0.0, min(al + aw, bl + bw) - max(al, bl))
        min_h = min(ah, bh)
        min_w = min(aw, bw)
        if min_h <= 0 or min_w <= 0:
            return False
        return (y_overlap / min_h) < 0.30 and (x_overlap / min_w) > 0.80

    def classify(sh, b):
        """Return (kind, label) or (None, None) if not a candidate."""
        _, _, w, h = b
        is_table = sh.shape_type == GRAPHIC_FRAME and getattr(sh, "has_table", False)
        is_chart = sh.shape_type == GRAPHIC_FRAME and getattr(sh, "has_chart", False)
        is_picture = sh.shape_type == PICTURE_TYPE
        is_auto_shape = sh.shape_type == AUTO_SHAPE
        has_text = sh.has_text_frame and sh.text_frame.text.strip()
        if is_table:
            return ("table", "Table")
        if is_chart:
            return ("chart", "Chart")
        if is_picture:
            return ("picture", sh.name)
        # AutoShape large enough to be a content block (chevron, anchor
        # block, KPI card body). Skip thin / small decorative shapes
        # (signature marks, edge strips, hairlines, corner blocks).
        if is_auto_shape and (
            w >= AUTO_SHAPE_MIN_EDGE
            and h >= AUTO_SHAPE_MIN_EDGE
            and w * h >= AUTO_SHAPE_MIN_AREA
        ):
            label = (sh.text_frame.text.strip()[:30] if has_text else
                     getattr(sh, "name", "Shape"))
            return ("shape", label.replace("\n", " "))
        if has_text and len(sh.text_frame.text.strip()) >= MIN_TEXT_LEN:
            return ("text", sh.text_frame.text.strip()[:40].replace("\n", " "))
        return (None, None)

    issues: list[tuple[int, str, str, float]] = []
    for idx, slide in enumerate(prs.slides, start=1):
        items: list[tuple[tuple[float, float, float, float], str, str]] = []
        for sh in slide.shapes:
            b = bbox(sh)
            if b is None:
                continue
            kind, label = classify(sh, b)
            if kind is None:
                continue
            items.append((b, kind, label))

        for i, (a_b, a_kind, a_label) in enumerate(items):
            for j, (b_b, b_kind, b_label) in enumerate(items):
                if i >= j:
                    continue  # canonical ordering — each pair once
                # Pair-type filter: keep pairs that risk content collision.
                # We flag overlaps that involve at least one "content"
                # shape (text, table, chart, picture, large AutoShape).
                # Skip pairs where both are the same kind of non-text
                # container (picture+picture, chart+chart) — those are
                # usually intentional side-by-side layout.
                kinds = {a_kind, b_kind}
                if kinds == {"picture"} or kinds == {"chart"} or kinds == {"table"}:
                    continue
                area = overlap_area(a_b, b_b)
                if area < THRESH_SQ_IN:
                    continue
                if mostly_contains(a_b, b_b) or mostly_contains(b_b, a_b):
                    continue
                # Layered design: one shape's center sits inside the
                # other (text-on-card, badge-on-image, etc.).
                if center_inside(a_b, b_b) or center_inside(b_b, a_b):
                    continue
                # Text-on-container: the visible text content sits near
                # the top of its bbox. If the text top-edge is inside
                # the other (card / image / shape), it's intentional
                # layering even when the textbox extends slightly past.
                if a_kind == "text" and text_top_inside(a_b, b_b):
                    continue
                if b_kind == "text" and text_top_inside(b_b, a_b):
                    continue
                # Vertically stacked text pairs (eyebrow + title, stacked
                # cards) — bbox overlap is geometric only; rendered text
                # sits at the top of each box and never collides.
                if a_kind == "text" and b_kind == "text" and is_vertical_stack(a_b, b_b):
                    continue
                issues.append((idx, a_label, b_label, round(area, 2)))
    return issues


def check_chart_lang(prs) -> list[tuple[int, str]]:
    """Flag chart XML with `<a:endParaRPr>` missing the `lang` attribute.

    python-pptx's chart templates are inconsistent: bar/line/scatter
    emit `<a:endParaRPr lang="en-US"/>`, but doughnut/pie emit
    `<a:endParaRPr/>` (no `lang`). Strict-mode PowerPoint rejects the
    latter — the recovery dialog fires and the chart is stripped,
    leaving an apparently-blank slide. Patch chart XML with
    `patch_chart_lang()` (see SKILL.md Charts section) before saving.

    Returns (slide_idx, chart_name).
    """
    issues: list[tuple[int, str]] = []
    for idx, slide in enumerate(prs.slides, start=1):
        for sh in slide.shapes:
            if not getattr(sh, "has_chart", False):
                continue
            chart_xml = sh.chart._chartSpace
            for ep in chart_xml.iter(qn("a:endParaRPr")):
                if ep.get("lang") is None:
                    issues.append((idx, sh.name))
                    break
    return issues


# --- top-level ----------------------------------------------------------------

def verify(pptx_path: str | Path) -> dict[str, Any]:
    """Run every check on a built .pptx. Returns a dict of issues.

    An empty list (or "ok"-like dict) per key means that check passed.
    A non-empty result is a hint — the model should inspect the listed
    slides visually (Step 3 PNG inspection) and decide whether to fix.
    """
    prs = Presentation(str(pptx_path))
    return {
        "slide_count": len(prs.slides),
        "palette": check_palette(prs),
        "fonts": check_fonts(prs),
        "sizes": check_size_discipline(prs),
        "overflow": check_overflow(prs),
        "page_numbers": check_page_numbers(prs),
        "overlap": check_overlap(prs),
        "chart_lang": check_chart_lang(prs),
    }


def summarize(issues: dict[str, Any]) -> str:
    """One-line summary for quick read."""
    n = issues["slide_count"]
    p = issues["palette"]
    flags = []
    if p["matched"] not in {"none"} and p["coverage"] < 0.85:
        flags.append(f"palette drift ({p['coverage']:.0%})")
    if issues["fonts"]:
        flags.append(f"{len(issues['fonts'])} CJK runs missing EA typeface")
    if issues["sizes"]:
        flags.append(f"{len(issues['sizes'])} slides with 5+ scale sizes")
    if issues["overflow"]:
        flags.append(f"{len(issues['overflow'])} fixed-size text boxes clip")
    if issues["page_numbers"]:
        flags.append(f"{len(issues['page_numbers'])} page-number violations")
    if issues.get("overlap"):
        flags.append(f"{len(issues['overlap'])} shape overlaps")
    if issues.get("chart_lang"):
        flags.append(
            f"{len(issues['chart_lang'])} charts missing endParaRPr lang "
            "(strict PowerPoint strips them)"
        )
    if not flags:
        return f"{n} slides — all checks passed ({p['matched']})."
    return f"{n} slides — issues: {' · '.join(flags)}"


if __name__ == "__main__":
    import json
    import sys

    if len(sys.argv) != 2:
        print("usage: verify_pptx.py <path-to-pptx>", file=sys.stderr)
        sys.exit(2)
    result = verify(sys.argv[1])
    print(summarize(result))
    print(json.dumps(result, indent=2, ensure_ascii=False, default=str))
