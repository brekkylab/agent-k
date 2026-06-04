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
            except Exception:
                pass
            if shape.has_text_frame:
                for para in shape.text_frame.paragraphs:
                    for run in para.runs:
                        try:
                            rgb = run.font.color.rgb
                            if rgb is not None:
                                out.append(str(rgb).upper())
                        except Exception:
                            pass
    return out


# --- checks -------------------------------------------------------------------

def check_palette(prs) -> dict[str, Any]:
    """Detect which gallery palette is in use, and report off-gallery colors.

    Returns {"matched": <name>|"custom"|"none", "coverage": float,
              "off_gallery": [hex, ...]}.
    A deck is "compliant" if coverage >= 0.85 against one gallery palette.
    Off-gallery colors are returned so the model can decide whether they're
    intentional accents or accidental drift.
    """
    fills = _collect_fills(prs)
    if not fills:
        return {"matched": "none", "coverage": 0.0, "off_gallery": []}

    used = set(fills) - NEUTRAL_TOLERATED
    best_name = "custom"
    best_coverage = 0.0
    best_off: set[str] = used

    for name, palette in GALLERY.items():
        if not used:
            break
        in_palette = used & palette
        coverage = len(in_palette) / len(used) if used else 0.0
        if coverage > best_coverage:
            best_coverage = coverage
            best_name = name
            best_off = used - palette

    if best_coverage < 0.5:
        # Couldn't match any gallery palette well — fully custom
        return {
            "matched": "custom",
            "coverage": round(best_coverage, 2),
            "off_gallery": sorted(used),
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
    """Flag slides that pack 5+ sizes from the four-step type scale.

    The scale tiers (with tolerance ±30%):
        Caption  ~11pt   (8-15)
        Body     ~18pt   (16-24)
        Title    ~32pt   (26-44)
        Display  ~54pt   (45-90)
    Dramatic-beat sizes (≥100pt or non-tier between 90-100) don't count.
    Returns list of (slide_idx, sizes_found_in_scale) where the count is 5+.
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
        scale_sizes: set[int] = set()
        for _shape, _para, run in _iter_runs(slide):
            try:
                if run.font.size:
                    pt = round(run.font.size.pt)
                    if tier(pt) is not None:
                        scale_sizes.add(pt)
            except Exception:
                pass
        if len(scale_sizes) >= 5:
            issues.append((i, sorted(scale_sizes)))
    return issues


def check_overflow(prs) -> list[tuple[int, str, str]]:
    """Estimate text-box overflow with CJK-aware width.

    A text box overflows when the estimated rendered height exceeds the
    box's height. Korean glyphs count as 2 ASCII widths. For shapes with
    auto_size=SHAPE_TO_FIT_TEXT, undersized boxes will *grow*, which
    often causes overlap with neighboring shapes — these are reported as
    "may overlap" rather than outright overflow.

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
            except Exception:
                continue
            if w_in <= 0 or h_in <= 0:
                continue
            autosize_grows = False
            try:
                if shape.text_frame.auto_size and "SHAPE_TO_FIT_TEXT" in str(
                    shape.text_frame.auto_size
                ):
                    autosize_grows = True
            except Exception:
                pass

            total_h = 0.0
            sample = ""
            for para in shape.text_frame.paragraphs:
                run_texts = [r.text for r in para.runs if r.text]
                if not run_texts:
                    continue
                text = "".join(run_texts)
                font_pt = 14
                for r in para.runs:
                    try:
                        if r.font.size:
                            font_pt = max(font_pt, round(r.font.size.pt))
                    except Exception:
                        pass
                units = sum(2 if ord(ch) > 0x7F else 1 for ch in text)
                chars_per_line = max(1, int(w_in * 12 * 14 / font_pt))
                lines = max(1, -(-units // chars_per_line))
                total_h += lines * font_pt * 1.4 / 72
                if not sample:
                    sample = text.strip()[:40]
            # Threshold: flag only "real" overflows, not minor wraps.
            # Fixed-size box: even small overflow is a clip (>0.1" or 25%).
            # Autosize box: only flag if grow is large enough to risk
            # overlapping neighbors (>0.15" absolute AND >25% relative).
            grow_abs = total_h - h_in
            grow_ratio = total_h / h_in if h_in > 0 else 1
            if autosize_grows:
                # Tightly-stacked bullets / cards can overlap with even
                # small grows. Catch any grow >25% — false positives are
                # cheaper than missed overlaps.
                if grow_ratio > 1.25 and grow_abs > 0.1:
                    issues.append((
                        i, sample,
                        f"autosize grow {h_in:.2f}\" → ~{total_h:.2f}\" "
                        f"(+{grow_abs:.2f}\") — may overlap neighbors"
                    ))
            else:
                if grow_abs > 0.1 or grow_ratio > 1.25:
                    issues.append((
                        i, sample,
                        f"text needs {total_h:.2f}\" but box is {h_in:.2f}\""
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

    def looks_like_page_num(text: str) -> bool:
        t = text.strip()
        return bool(
            re.fullmatch(r"\d{1,2}\s*/\s*\d{1,2}", t)
            or re.fullmatch(r"0?\d{1,2}\s*·\s*\d{1,2}", t)
        )

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


def check_picture_overrun(prs) -> list[tuple[int, str, str, float]]:
    """Flag Picture shapes whose bbox overlaps a non-container TextBox.

    Returns (slide_idx, picture_name, textbox_preview, overlap_sq_in)
    for every (Picture, TextBox) pair whose bbox intersection exceeds
    0.1 sq inches. Container-content overlaps (a Picture entirely
    *containing* the TextBox, or vice-versa) are NOT flagged — those
    are intentional layering (e.g. caption inside a card).
    """
    THRESH_SQ_IN = 0.1
    PICTURE_TYPE = 13  # MSO_SHAPE_TYPE.PICTURE

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

    def fully_contains(outer, inner):
        ol, ot, ow, oh = outer
        il, it, iw, ih = inner
        return (
            ol <= il and ot <= it
            and ol + ow >= il + iw and ot + oh >= it + ih
        )

    issues: list[tuple[int, str, str, float]] = []
    for idx, slide in enumerate(prs.slides, start=1):
        pics = []
        texts = []
        for sh in slide.shapes:
            b = bbox(sh)
            if b is None:
                continue
            if sh.shape_type == PICTURE_TYPE:
                pics.append((sh, b))
            elif sh.has_text_frame and sh.text_frame.text.strip():
                texts.append((sh, b))
        for pic, pb in pics:
            for tb, tbb in texts:
                area = overlap_area(pb, tbb)
                if area < THRESH_SQ_IN:
                    continue
                # skip intentional containment (caption-on-picture etc.)
                if fully_contains(pb, tbb) or fully_contains(tbb, pb):
                    continue
                preview = tb.text_frame.text[:40].replace("\n", " ")
                issues.append((idx, pic.name, preview, round(area, 2)))
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
        "picture_overrun": check_picture_overrun(prs),
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
        flags.append(f"{len(issues['overflow'])} text boxes overflow")
    if issues["page_numbers"]:
        flags.append(f"{len(issues['page_numbers'])} page-number violations")
    if issues.get("picture_overrun"):
        flags.append(f"{len(issues['picture_overrun'])} picture-text overlaps")
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
