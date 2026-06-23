"""verify_pptx — design-system compliance checks for a built .pptx.

Usage (inside the sandbox):
    import sys
    sys.path.insert(0, "/workspace/skills/pptx/script")
    from verify_pptx import verify, summarize

    issues = verify("/workspace/artifacts/deck.pptx")
    print(summarize(issues))

`verify()` returns a dict with `slide_count` plus five check keys:
`palette`, `fonts`, `sizes`, `page_numbers`, `overlap`. An empty list
per check = passed.
Heuristic: catches the most-violated SKILL.md rules, not every nuance.

Decks are built by html2pptx.py, which DECOMPOSES each slide into native
objects: editable text boxes (with FROZEN line breaks — `word_wrap=False`
+ hard `<a:br/>`, so the viewer can't re-wrap CJK), native autoshapes for
simple boxes/borders/markers, a picture per chart/image, and a native
solid slide-background fill. `fonts`/`sizes`/`page_numbers`/`overlap`
apply to the text boxes; `palette` checks DISCIPLINE — that the deck uses a
small, locked set of colors (composed per deck, not a fixed gallery), not
that it matches a known palette. Advisory; lean on visual inspection for
whether the palette fits the subject.

There is no `overflow` check (boxes are sized to browser-measured text,
and frozen lines can't re-wrap), no `word_wrap` check (`word_wrap=False`
is the intended design everywhere), and no native-chart check (charts
are raster pictures).
"""

from __future__ import annotations

from pathlib import Path
from typing import Any

from pptx import Presentation
from pptx.oxml.ns import qn


# Pure white / black are neutrals, not brand colours; excluded from the
# palette-discipline count below.
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


def _slide_colors(slide) -> set[str]:
    """Distinct non-neutral solid-fill + text-run colors on one slide."""
    cols: set[str] = set()
    for shape in slide.shapes:
        try:
            if shape.fill.type == 1:  # solid
                h = str(shape.fill.fore_color.rgb).upper()
                if h not in NEUTRAL_TOLERATED:
                    cols.add(h)
        except (AttributeError, TypeError):
            pass  # no fill / theme fill / placeholder
        if shape.has_text_frame:
            for para in shape.text_frame.paragraphs:
                for run in para.runs:
                    try:
                        rgb = run.font.color.rgb
                        if rgb is not None:
                            h = str(rgb).upper()
                            if h not in NEUTRAL_TOLERATED:
                                cols.add(h)
                    except (AttributeError, TypeError):
                        pass  # inherits theme color
    return cols


# --- checks -------------------------------------------------------------------

# A composed palette runs ~7-10 non-neutral colors (primary/secondary/accent/
# muted/hairline/positive/negative, plus a tinted bg/surface and maybe a chart
# tint — only pure #FFFFFF/#000000 are treated as neutral). Past this it reads
# as palette sprawl, not a locked palette. Kept generous so a legitimately
# composed warm/tinted palette isn't flagged.
PALETTE_LIMIT = 12


def check_palette(prs) -> dict[str, Any]:
    """Palette DISCIPLINE — a composed deck should use a small, locked set of
    colors, consistent across slides. (There is no fixed gallery to match;
    palettes are composed per deck from the subject — see SKILL.md.)

    Collects distinct non-neutral solid-fill + text-run colors per slide.
    Reports the distinct count, the colors, and any `stray` color that appears
    on only ONE slide (a new hex introduced mid-deck). Compliant when the
    distinct count is small (<= PALETTE_LIMIT). Advisory — visual inspection
    judges whether the palette actually fits the subject.

    Returns {"distinct": int, "colors": [hex,...], "stray": [hex,...],
              "ok": bool}.
    """
    from collections import Counter

    n_slides = len(prs.slides)
    appears_on: Counter = Counter()
    for slide in prs.slides:
        for h in _slide_colors(slide):
            appears_on[h] += 1

    distinct = sorted(appears_on)
    # A color on a single slide of a multi-slide deck = introduced mid-deck.
    stray = sorted(h for h, c in appears_on.items() if c == 1) if n_slides >= 3 else []
    return {
        "distinct": len(distinct),
        "colors": distinct,
        "stray": stray,
        "ok": len(distinct) <= PALETTE_LIMIT,
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
    return issues


def check_size_discipline(prs) -> list[tuple[int, list[int]]]:
    """Flag slides that mix too many tier / out-of-tier sizes.

    The scale is composed per deck (SKILL.md "Type scale"), so tiers are wide
    bands covering its ranges: caption ≤11, body 12-20, title 21-35, display
    36-90 pt. Sizes in the same tier collapse to one; out-of-tier sizes count
    individually, except the first off-scale size per slide is a permitted
    dramatic beat (hero number / divider numeral / quote glyph) and is not
    counted. Flag at 5+ classes per slide — with only four tiers, that needs
    all four plus 2+ off-scale sizes.

    Returns list of (slide_idx, sizes_found_on_slide).
    """

    def tier(pt: int) -> str | None:
        if pt <= 0:
            return None
        if pt <= 11:
            return "caption"
        if pt <= 20:
            return "body"
        if pt <= 35:
            return "title"
        if pt <= 90:
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
        # One off-scale size per slide is an allowed dramatic beat (hero
        # number, divider numeral, quote glyph) — don't count the first.
        classes = len(tiers_used) + max(0, len(out_of_tier) - 1)
        if classes >= 5:
            issues.append((i, sorted(all_sizes)))
    return issues


def check_page_numbers(prs) -> list[tuple[int, str]]:
    """Title (slide 1) and closing (slide N) must not carry a page number.

    Divider detection is deferred to the model — too easy to mis-classify
    a content slide as a divider from XML alone.

    Returns list of (slide_idx, reason).
    """
    import re

    n = len(prs.slides)
    if n == 0:
        return []

    # Page-number conventions matched (fullmatch — whole text is the number):
    #   "3 / 10", "3/10", "3·10"   → n[/·]N (1-3 digits each)
    #   "Page 3", "Page 03"        → "Page n" (case-insensitive)
    #   "3 of 10"                  → "n of N"
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
    return issues


def check_overlap(prs) -> list[tuple[int, str, str, float]]:
    """Flag visually colliding shape pairs on each slide.

    Considers pairs drawn from: text-box, table, chart, picture, and
    large AutoShape (chevron / anchor block / card body — thin
    decorative shapes are filtered out by edge/area thresholds). Skips
    same-kind container pairs (picture+picture, chart+chart, table+
    table) since those are usually intentional side-by-side layout.

    Layered-design pairs are filtered out: one shape fully containing
    the other, one shape's center inside the other's bbox, a text box
    whose top-edge sits inside its container, or two vertically
    stacked text boxes (eyebrow + title, stacked card descriptions).

    Each (shape A, shape B) pair is reported once.

    Returns (slide_idx, shape_A_label, shape_B_label, overlap_sq_in).
    """
    THRESH_SQ_IN = 0.2
    MIN_TEXT_LEN = 10
    PICTURE_TYPE = 13       # MSO_SHAPE_TYPE.PICTURE
    AUTO_SHAPE = 1          # MSO_SHAPE_TYPE.AUTO_SHAPE (chevrons, rounded rects, etc.)
    # Tables and native charts are both `GraphicFrame` shapes but have
    # different MSO_SHAPE_TYPE values (TABLE=19 vs CHART=3). Detect via
    # the python-pptx `has_table` / `has_chart` attributes instead, which
    # work regardless of shape_type.
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
        # 0.45 tolerance absorbs the small CJK height cushion html2pptx
        # adds, which would otherwise flag eyebrow+title stacks as collisions.
        return (y_overlap / min_h) < 0.45 and (x_overlap / min_w) > 0.70

    def classify(sh, b):
        """Return (kind, label) or (None, None) if not a candidate."""
        _, _, w, h = b
        is_table = getattr(sh, "has_table", False)
        is_chart = getattr(sh, "has_chart", False)
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
        "page_numbers": check_page_numbers(prs),
        "overlap": check_overlap(prs),
    }


def summarize(issues: dict[str, Any]) -> str:
    """One-line summary for quick read."""
    n = issues["slide_count"]
    p = issues["palette"]
    flags = []
    if not p.get("ok", True):
        flags.append(f"palette sprawl ({p['distinct']} distinct colors)")
    if issues["fonts"]:
        flags.append(f"{len(issues['fonts'])} CJK runs missing EA typeface")
    if issues["sizes"]:
        flags.append(f"{len(issues['sizes'])} slides with 5+ scale sizes")
    if issues["page_numbers"]:
        flags.append(f"{len(issues['page_numbers'])} page-number violations")
    if issues.get("overlap"):
        flags.append(f"{len(issues['overlap'])} text overlaps")
    if not flags:
        return f"{n} slides — all checks passed ({p['distinct']} palette colors)."
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
