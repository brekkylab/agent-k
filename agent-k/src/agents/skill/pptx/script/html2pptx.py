"""html2pptx — author slides in HTML/CSS, emit an EDITABLE .pptx.

The deck is authored as HTML (one `.slide` element per slide, each
1280x720 px = 16:9) and DECOMPOSED into native PowerPoint objects rather
than flattened into one image:

  - **Text blocks**  → native, editable text boxes.
  - **Simple shapes** (solid-fill boxes, borders, hairline dividers,
    edge strips, card top-strips, bullet markers) → native autoshapes
    (rectangles / rounded rectangles / lines) you can move and recolor.
  - **Charts / images / SVG** (Chart.js `<canvas>`, `<img>`, inline
    `<svg>`) → each its OWN picture object (separate, movable), not
    fused into the slide. Their internal text is not editable (raster).
  - **Page backdrop + anything not otherwise handled** → one full-bleed
    base picture behind everything (a safety net so nothing vanishes).

Stacking follows document order (backdrop → shapes/pictures in DOM order
→ text on top), so "photo behind cards behind chart, text on top" works.

Usage (inside the sandbox):
    pip install python-pptx playwright
    playwright install chromium --only-shell
    python html2pptx.py deck.html -o /workspace/artifacts/deck.pptx

px->inch is exact at 96 px/in (1280x720 = 13.333"x7.5"). Do NOT use CSS
`transform: scale()` / `zoom` on slide content — it breaks the mapping.
"""

from __future__ import annotations

import argparse
import json
import sys
import tempfile
from pathlib import Path

EMU_PER_PX = 9525            # 914400 EMU/in ÷ 96 px/in
SLIDE_W_PX = 1280
SLIDE_H_PX = 720

# Slack so soffice/PowerPoint font-metric drift (vs Chromium) doesn't
# re-wrap and clip text. Width slack is centered; height grows down only.
PAD_W_PX = 8
PAD_H_PX = 8

def _clip(l, t, w, h, warn=None, slide_idx=None, kind=""):
    """Clip an element's px rect to the slide bounds (the browser hides
    overflow via `.slide{overflow:hidden}`; without this, off-canvas
    elements render INTO the slide in pptx — "punching through" the edge).
    Returns the clipped (l,t,w,h) or None if fully off-canvas. Records a
    warning when the element overflowed a bound by more than 16 px so the
    author can fix the layout (a clipped card still looks cut)."""
    r, b = l + w, t + h
    cl, ct, cr, cb = max(0.0, l), max(0.0, t), min(SLIDE_W_PX, r), min(SLIDE_H_PX, b)
    if warn is not None:
        over = max(-l, -t, r - SLIDE_W_PX, b - SLIDE_H_PX)
        if over > 16:
            warn.append((slide_idx, kind, round(over)))
    if cr <= cl or cb <= ct:
        return None
    return (cl, ct, cr - cl, cb - ct)


# --- in-page helpers ----------------------------------------------------------

# Synthesize real <span> elements for absolutely-positioned ::before/::after
# pseudo-markers (e.g. the bullet dots in components.css) so they become real
# DOM nodes the shape scraper can promote to native autoshapes, then suppress
# the original pseudo-element so it doesn't double in the base picture.
INJECT_MARKERS_JS = r"""
() => {
  const kill = document.createElement('style');
  kill.textContent =
    '.pptx-kill-before::before{content:none !important}' +
    '.pptx-kill-after::after{content:none !important}';
  document.head.appendChild(kill);
  const opaque = (c) => c && c !== 'transparent' &&
    !/rgba?\([^)]*,\s*0(\.0+)?\s*\)/.test(c);
  for (const el of document.querySelectorAll('.slide *')) {
    for (const pe of ['::before', '::after']) {
      const s = getComputedStyle(el, pe);
      if (!s) continue;
      const w = parseFloat(s.width), h = parseFloat(s.height);
      if (s.position !== 'absolute' || !opaque(s.backgroundColor)) continue;
      if (!(w > 0 && h > 0)) continue;
      if (getComputedStyle(el).position === 'static') el.style.position = 'relative';
      const span = document.createElement('span');
      span.style.cssText =
        `position:absolute;left:${s.left};top:${s.top};width:${s.width};` +
        `height:${s.height};background:${s.backgroundColor};` +
        `border-radius:${s.borderTopLeftRadius};pointer-events:none;`;
      span.setAttribute('data-pptx-synth', '1');
      el.appendChild(span);
      el.classList.add(pe === '::before' ? 'pptx-kill-before' : 'pptx-kill-after');
    }
  }
}
"""

# Per-slide scraper. Marks pictures (data-pptx-pic=idx), shapes
# (data-pptx-shape), and text blocks (data-pptx-text); returns geometry +
# styles. `order` is document order, used for z-stacking on emit.
SCRAPE_SLIDE_JS = r"""
([slide, picBase]) => {
  const sb = slide.getBoundingClientRect();
  const rel = (r) => ({ x: r.left - sb.left, y: r.top - sb.top, w: r.width, h: r.height });

  const rgba = (c) => {
    const m = (c || '').match(/rgba?\(([^)]+)\)/);
    if (!m) return null;
    const p = m[1].split(',').map(s => parseFloat(s.trim()));
    const a = p.length >= 4 ? p[3] : 1;
    return { hex: p.slice(0, 3).map(v => Math.round(v).toString(16).padStart(2, '0')).join('').toUpperCase(), a };
  };
  const visible = (el) => {
    const s = getComputedStyle(el);
    return !(s.display === 'none' || s.visibility === 'hidden' || parseFloat(s.opacity) === 0);
  };
  const ancestorsVisible = (el) => {
    let a = el.parentElement;
    while (a && a !== slide) { if (!visible(a)) return false; a = a.parentElement; }
    return true;
  };
  const isSvgInternal = (el) => !!el.ownerSVGElement;        // inside an <svg>
  // NB: an inline <svg> reports tagName "svg" (lowercase, not HTML-uppercased),
  // so compare case-insensitively or SVG charts are silently dropped.
  const isMedia = (el) => ['CANVAS', 'IMG', 'SVG'].includes((el.tagName || '').toUpperCase()) && !el.ownerSVGElement;

  const pics = [], shapes = [], texts = [];
  let order = 0;
  let picIdx = picBase;

  // Slide backdrop → emitted as a NATIVE solid slide-background fill (no
  // full-slide "frame" picture). A base picture is only emitted as a
  // fallback when the slide carries something we can't model natively
  // (gradient / background-image / shadow): hasComplex flags that.
  const scs = getComputedStyle(slide);
  const sbg = rgba(scs.backgroundColor);
  const bgHex = sbg && sbg.a > 0.3 ? sbg.hex : 'FFFFFF';
  let hasComplex = scs.backgroundImage !== 'none' || scs.boxShadow !== 'none';

  // Single document-order walk over the slide subtree.
  const all = slide.querySelectorAll('*');
  const cand = new Set();                                    // text-block candidates
  for (const el of all) {
    if (isSvgInternal(el)) continue;
    for (const n of el.childNodes)
      if (n.nodeType === 3 && n.textContent.trim()) { cand.add(el); break; }
  }

  for (const el of all) {
    order++;
    if (isSvgInternal(el)) continue;
    if (!visible(el) || !ancestorsVisible(el)) continue;
    const r = el.getBoundingClientRect();
    if (r.width <= 0 || r.height <= 0) continue;

    // 1) Media → its own picture object.
    if (isMedia(el)) {
      el.setAttribute('data-pptx-pic', String(picIdx));
      pics.push({ order, idx: picIdx, ...rel(r) });
      picIdx++;
      continue;
    }

    const cs = getComputedStyle(el);

    // 2) Simple shape (solid fill and/or solid borders). Skip when the box
    //    carries a gradient/background-image or shadow — too complex to model;
    //    it falls through to the base fallback picture instead.
    const complexPaint = cs.backgroundImage !== 'none' || cs.boxShadow !== 'none';
    if (complexPaint) hasComplex = true;
    if (el !== slide && !complexPaint && parseFloat(cs.opacity) >= 0.99) {
      const bg = rgba(cs.backgroundColor);
      const fill = bg && bg.a > 0.1 ? bg.hex : null;
      const sides = [];
      for (const sd of ['Top', 'Right', 'Bottom', 'Left']) {
        const w = parseFloat(cs['border' + sd + 'Width']) || 0;
        const st = cs['border' + sd + 'Style'];
        const col = rgba(cs['border' + sd + 'Color']);
        if (w > 0.4 && st !== 'none' && st !== 'hidden' && col && col.a > 0.1)
          sides.push({ side: sd.toLowerCase(), w, color: col.hex });
      }
      let border = null;
      if (sides.length === 4 && sides.every(s =>
            Math.round(s.w) === Math.round(sides[0].w) && s.color === sides[0].color))
        border = { kind: 'all', w: sides[0].w, color: sides[0].color };
      else if (sides.length)
        border = { kind: 'sides', sides };

      if (fill || border) {
        el.setAttribute('data-pptx-shape', '1');
        shapes.push({
          order, ...rel(r), fill, border,
          radius: parseFloat(cs.borderTopLeftRadius) || 0,
        });
      }
    }

    // 3) Text block (topmost text-bearing element in its chain).
    if (cand.has(el)) {
      let anc = el.parentElement, nested = false;
      while (anc && anc !== slide) { if (cand.has(anc)) { nested = true; break; } anc = anc.parentElement; }
      if (!nested) {
        // FREEZE the browser's line breaks: split each text node at its
        // rendered line boundaries (the y-top of a 1-char range changes at a
        // wrap), so PowerPoint/soffice can't re-wrap CJK differently. Emit
        // per-line run groups; the emitter joins them with hard <a:br>.
        const mkStyle = (se) => {
          const s = getComputedStyle(se);
          return {
            font: (s.fontFamily.split(',')[0] || '').replace(/["']/g, '').trim(),
            size_pt: parseFloat(s.fontSize) * 0.75,
            bold: (parseInt(s.fontWeight) || 400) >= 600,
            italic: s.fontStyle === 'italic',
            color: (rgba(s.color) || {}).hex || null,
            letter_spacing: s.letterSpacing === 'normal' ? 0 : parseFloat(s.letterSpacing) * 0.75,
          };
        };
        let bb = null;
        const grow = (rr) => {
          if (rr.width === 0 && rr.height === 0) return;
          bb = bb ? {
            left: Math.min(bb.left, rr.left), top: Math.min(bb.top, rr.top),
            right: Math.max(bb.right, rr.right), bottom: Math.max(bb.bottom, rr.bottom),
          } : { left: rr.left, top: rr.top, right: rr.right, bottom: rr.bottom };
        };
        // Two vertical spans are on the same visual line when they overlap by
        // more than half the shorter span's height. Same-line size mixes overlap
        // ~fully (~1.0); even tight-leading wraps overlap little (≲0.15).
        const samELine = (t1, b1, t2, b2) => {
          const ov = Math.min(b1, b2) - Math.max(t1, t2);
          const minH = Math.min(b1 - t1, b2 - t2);
          return minH > 0 && ov > 0.5 * minH;
        };
        const segs = [];   // ordered {text, top, bottom, style}
        const addText = (tn, styleEl) => {
          const s = tn.textContent;
          if (!s) return;
          const col0 = rgba(getComputedStyle(styleEl).color);
          if (col0 && col0.a < 0.1) return;   // fully-transparent text → no text
          const st = mkStyle(styleEl);
          const rg = document.createRange();
          let lineStart = 0, curTop = null, curBot = null;
          for (let i = 0; i < s.length; i++) {
            rg.setStart(tn, i); rg.setEnd(tn, i + 1);
            const rects = rg.getClientRects();
            if (!rects.length) continue;
            grow(rects[0]);
            const top = rects[0].top, bot = rects[0].bottom;
            if (curTop === null) { curTop = top; curBot = bot; }
            // A real wrap drops to a new line, so the glyph barely overlaps the
            // current line vertically. A different font size on the SAME line
            // overlaps heavily (the small run sits inside the tall one), so we
            // split on overlap FRACTION, not on a raw top change — that keeps a
            // big number + small unit on one line yet still catches tight-leading
            // wraps that a plain band-overlap test would wrongly merge.
            else if (samELine(top, bot, curTop, curBot)) {
              curTop = Math.min(curTop, top); curBot = Math.max(curBot, bot);
            } else {
              segs.push({ text: s.slice(lineStart, i), top: curTop, bottom: curBot, style: st });
              lineStart = i; curTop = top; curBot = bot;
            }
          }
          segs.push({ text: s.slice(lineStart), top: curTop === null ? 0 : curTop,
                      bottom: curBot === null ? 0 : curBot, style: st });
        };
        for (const n of el.childNodes) {
          if (n.nodeType === 3) { if (n.textContent.trim()) addText(n, el); }
          else if (n.nodeType === 1 && !n.hasAttribute('data-pptx-synth')) {
            let any = false;
            for (const m of n.childNodes)
              if (m.nodeType === 3 && m.textContent.trim()) { addText(m, n); any = true; }
            if (!any && n.textContent.trim()) {
              const rr = n.getBoundingClientRect(); grow(rr);
              segs.push({ text: n.textContent, top: Math.round(rr.top),
                          bottom: Math.round(rr.bottom), style: mkStyle(n) });
            }
          }
        }
        if (segs.length && bb) {
          // Group segments into visual lines by the same overlap-fraction test,
          // then merge adjacent same-style runs. A big number + a small unit
          // span share a line (heavy overlap); a real wrap starts a new one.
          const rawLines = []; let cur = null, curTop = null, curBot = null;
          for (const sg of segs) {
            if (cur !== null && samELine(sg.top, sg.bottom, curTop, curBot)) {
              curTop = Math.min(curTop, sg.top); curBot = Math.max(curBot, sg.bottom);
            } else {
              cur = []; rawLines.push(cur); curTop = sg.top; curBot = sg.bottom;
            }
            const last = cur[cur.length - 1];
            if (last && JSON.stringify(last.style) === JSON.stringify(sg.style)) last.text += sg.text;
            else cur.push({ text: sg.text, style: sg.style });
          }
          const lines = [];
          for (const ln of rawLines) {
            const runs = ln.map(rn => ({ text: rn.text.replace(/\s+/g, ' '), ...rn.style }))
                           .filter(rn => rn.text.length);
            if (runs.length) { runs[0].text = runs[0].text.replace(/^ /, ''); runs[runs.length - 1].text = runs[runs.length - 1].text.replace(/ $/, ''); }
            if (runs.some(rn => rn.text.trim())) lines.push({ runs });
          }
          if (lines.length) {
            el.setAttribute('data-pptx-text', '1');
            const fs = parseFloat(cs.fontSize) || 16;
            texts.push({
              order, x: bb.left - sb.left, y: bb.top - sb.top, w: bb.right - bb.left, h: bb.bottom - bb.top,
              align: cs.textAlign === 'start' ? 'left' : cs.textAlign === 'end' ? 'right' : cs.textAlign,
              line_height: cs.lineHeight === 'normal' ? 1.2 : parseFloat(cs.lineHeight) / fs,
              lines,
            });
          }
        }
      }
    }
  }
  return { w: sb.width, h: sb.height, pics, shapes, texts, nextPic: picIdx, bgHex, hasComplex };
}
"""

# Visibility control injected as a single mutable <style id=pptx-ctl>.
MODE_STYLES = {
    # base picture: hide everything we emit natively, keep slide backdrop
    "base": "[data-pptx-shape]{background-color:transparent !important;border-color:transparent !important}"
            "[data-pptx-text]{color:transparent !important;text-shadow:none !important}"
            "[data-pptx-pic]{visibility:hidden !important}",
    # picture pass: media visible & isolated (transparent backdrop), shapes/text hidden
    "pics": "[data-pptx-shape]{background-color:transparent !important;border-color:transparent !important}"
            "[data-pptx-text]{color:transparent !important;text-shadow:none !important}"
            ".slide,body{background:transparent !important}",
}
SET_MODE_JS = """
(css) => {
  let el = document.getElementById('pptx-ctl');
  if (!el) { el = document.createElement('style'); el.id = 'pptx-ctl'; document.head.appendChild(el); }
  el.textContent = css;
}
"""


def render_and_scrape(html_path: Path, png_dir: Path, scale: int) -> list[dict]:
    from playwright.sync_api import sync_playwright

    url = html_path.resolve().as_uri()
    slides: list[dict] = []
    with sync_playwright() as pw:
        browser = pw.chromium.launch()
        page = browser.new_page(
            viewport={"width": SLIDE_W_PX, "height": SLIDE_H_PX},
            device_scale_factor=scale,
        )
        page.goto(url, wait_until="networkidle")
        try:
            page.evaluate("document.fonts.ready")
        except Exception:
            pass
        try:
            page.wait_for_function(
                "window.__pptx_ready === undefined || window.__pptx_ready === true",
                timeout=5000,
            )
        except Exception:
            pass
        page.wait_for_timeout(400)

        page.evaluate(INJECT_MARKERS_JS)

        handles = page.query_selector_all(".slide")
        if not handles:
            raise SystemExit(
                'no `.slide` elements found — wrap each slide in '
                '<div class="slide">…</div> sized 1280x720 px.'
            )

        # Pass 1: scrape + mark every slide (assign global picture indices).
        pic_base = 0
        for h in handles:
            data = page.evaluate(SCRAPE_SLIDE_JS, [h, pic_base])
            pic_base = data.pop("nextPic")
            slides.append(data)

        # Pass 2: base fallback picture — ONLY for slides with un-modelable
        # content (gradient / background-image / shadow). Solid-color slides
        # get a native background fill instead (no full-slide "frame" image).
        if any(s.get("hasComplex") for s in slides):
            page.evaluate(SET_MODE_JS, MODE_STYLES["base"])
            page.wait_for_timeout(80)
            for i, h in enumerate(handles, 1):
                if not slides[i - 1].get("hasComplex"):
                    continue
                png = png_dir / f"base-{i}.png"
                h.scroll_into_view_if_needed()
                h.screenshot(path=str(png))
                slides[i - 1]["background"] = str(png)

        # Pass 3: each media element as its own isolated picture.
        page.evaluate(SET_MODE_JS, MODE_STYLES["pics"])
        page.wait_for_timeout(80)
        for ph in page.query_selector_all("[data-pptx-pic]"):
            idx = ph.get_attribute("data-pptx-pic")
            png = png_dir / f"pic-{idx}.png"
            try:
                ph.scroll_into_view_if_needed()
                ph.screenshot(path=str(png), omit_background=True)
            except Exception:
                png = None
            for sd in slides:
                for p in sd["pics"]:
                    if str(p["idx"]) == str(idx):
                        p["png"] = str(png) if png else None
        browser.close()
    return slides


def _set_ea(run, typeface: str) -> None:
    """Set Latin + East-Asian + complex-script typefaces on a run (CJK safety)."""
    from pptx.oxml.ns import qn

    rPr = run._r.get_or_add_rPr()
    for tag in ("latin", "ea", "cs"):
        el = rPr.find(qn(f"a:{tag}"))
        if el is None:
            el = rPr.makeelement(qn(f"a:{tag}"), {})
            rPr.append(el)
        el.set("typeface", typeface)


def build_pptx(slides: list[dict], out_path: Path) -> None:
    from pptx import Presentation
    from pptx.dml.color import RGBColor
    from pptx.enum.shapes import MSO_SHAPE
    from pptx.enum.text import MSO_ANCHOR, MSO_AUTO_SIZE, PP_ALIGN
    from pptx.util import Emu, Pt

    align_map = {"left": PP_ALIGN.LEFT, "center": PP_ALIGN.CENTER,
                 "right": PP_ALIGN.RIGHT, "justify": PP_ALIGN.JUSTIFY}

    def emu(px: float) -> Emu:
        return Emu(int(round(px * EMU_PER_PX)))

    prs = Presentation()
    prs.slide_width = emu(SLIDE_W_PX)
    prs.slide_height = emu(SLIDE_H_PX)
    blank = prs.slide_layouts[6]
    warnings: list = []

    def add_rect(slide, l, t, w, h, fill=None, line=None, line_w=0, radius=0, idx=None):
        c = _clip(l, t, w, h, warnings, idx, "shape")
        if c is None:
            return None
        l, t, w, h = c
        shp = slide.shapes.add_shape(
            MSO_SHAPE.ROUNDED_RECTANGLE if radius else MSO_SHAPE.RECTANGLE,
            emu(l), emu(t), emu(max(1, w)), emu(max(1, h)),
        )
        shp.shadow.inherit = False
        if fill:
            shp.fill.solid()
            shp.fill.fore_color.rgb = RGBColor.from_string(fill)
        else:
            shp.fill.background()
        if line:
            shp.line.color.rgb = RGBColor.from_string(line)
            shp.line.width = emu(max(0.75, line_w))
        else:
            shp.line.fill.background()
        if radius:
            try:
                shp.adjustments[0] = max(0.0, min(0.5, radius / max(1.0, min(w, h))))
            except Exception:
                pass
        return shp

    def emit_shape(slide, s, idx):
        l, t, w, h = s["x"], s["y"], s["w"], s["h"]
        fill, bd, rad = s.get("fill"), s.get("border"), s.get("radius") or 0
        uniform = bd and bd["kind"] == "all"
        if fill or uniform:
            add_rect(slide, l, t, w, h, fill=fill,
                     line=(bd["color"] if uniform else None),
                     line_w=(bd["w"] if uniform else 0), radius=rad, idx=idx)
        if bd and bd["kind"] == "sides":
            for sb in bd["sides"]:
                bw = max(1.0, sb["w"])
                if sb["side"] == "top":
                    add_rect(slide, l, t, w, bw, fill=sb["color"], idx=idx)
                elif sb["side"] == "bottom":
                    add_rect(slide, l, t + h - bw, w, bw, fill=sb["color"], idx=idx)
                elif sb["side"] == "left":
                    add_rect(slide, l, t, bw, h, fill=sb["color"], idx=idx)
                else:
                    add_rect(slide, l + w - bw, t, bw, h, fill=sb["color"], idx=idx)

    for si, sd in enumerate(slides, 1):
        slide = prs.slides.add_slide(blank)
        # Native solid slide background (no full-slide picture frame).
        bg = sd.get("bgHex")
        if bg:
            slide.background.fill.solid()
            slide.background.fill.fore_color.rgb = RGBColor.from_string(bg)
        # Fallback base picture only when the slide has un-modelable content.
        if sd.get("hasComplex") and sd.get("background"):
            slide.shapes.add_picture(sd["background"], 0, 0,
                                     width=prs.slide_width, height=prs.slide_height)

        # Shapes + pictures interleaved in document order (preserves stacking).
        objs = [("shape", s) for s in sd["shapes"]] + \
               [("pic", p) for p in sd["pics"] if p.get("png")]
        objs.sort(key=lambda o: o[1]["order"])
        for kind, o in objs:
            if kind == "shape":
                emit_shape(slide, o, si)
            else:
                c = _clip(o["x"], o["y"], o["w"], o["h"], warnings, si, "image")
                if c is None:
                    continue
                slide.shapes.add_picture(o["png"], emu(c[0]), emu(c[1]),
                                         width=emu(c[2]), height=emu(c[3]))

        # Text on top — frozen line breaks. word_wrap=False so the viewer
        # renders exactly the lines Chromium produced (no CJK re-wrap), with
        # hard <a:br> between captured lines. One editable box per block.
        for tb in sd["texts"]:
            align = tb.get("align")
            # Extend the small pad away from the anchored edge so left-aligned
            # body never shifts onto its bullet marker.
            if align == "right":
                x0 = tb["x"] - PAD_W_PX
            elif align in ("center", "justify"):
                x0 = tb["x"] - PAD_W_PX / 2
            else:
                x0 = tb["x"]
            c = _clip(x0, tb["y"], tb["w"] + PAD_W_PX, tb["h"] + PAD_H_PX, warnings, si, "text")
            if c is None:
                continue
            box = slide.shapes.add_textbox(emu(c[0]), emu(c[1]), emu(c[2]), emu(c[3]))
            box.shadow.inherit = False
            tf = box.text_frame
            tf.word_wrap = False
            tf.auto_size = MSO_AUTO_SIZE.NONE
            tf.margin_left = tf.margin_right = 0
            tf.margin_top = tf.margin_bottom = 0
            tf.vertical_anchor = MSO_ANCHOR.TOP
            p = tf.paragraphs[0]
            p.alignment = align_map.get(align, PP_ALIGN.LEFT)
            lh = tb.get("line_height")
            if lh and lh > 0:
                p.line_spacing = float(lh)
            first = True
            for ln in tb["lines"]:
                if not first:
                    p.add_line_break()
                first = False
                for r in ln["runs"]:
                    run = p.add_run()
                    run.text = r["text"]
                    if r.get("size_pt"):
                        run.font.size = Pt(r["size_pt"])
                    run.font.bold = bool(r.get("bold"))
                    run.font.italic = bool(r.get("italic"))
                    if r.get("color"):
                        run.font.color.rgb = RGBColor.from_string(r["color"])
                    fam = r.get("font") or "Arial"
                    run.font.name = fam
                    _set_ea(run, fam)
                    spc = r.get("letter_spacing")
                    if spc:
                        run._r.get_or_add_rPr().set("spc", str(int(round(spc * 100))))

    out_path.parent.mkdir(parents=True, exist_ok=True)
    prs.save(str(out_path))

    if warnings:
        worst: dict = {}
        for idx, kind, over in warnings:
            if over > worst.get(idx, (0, ""))[0]:
                worst[idx] = (over, kind)
        for idx in sorted(worst):
            over, kind = worst[idx]
            print(f"WARNING: slide {idx} has a {kind} overflowing the 1280x720 "
                  f"canvas by ~{over}px (clipped). Fix the layout so all content "
                  f"fits inside the slide.")


def _export_source_html(html_path: Path, out_path: Path) -> None:
    """Save a self-contained copy of the authored HTML next to the .pptx:
    inline any linked local stylesheet (e.g. components.css) into a <style>
    block so the file opens stand-alone. CDN <script> tags are left as-is.
    Useful for comparing what different models authored."""
    import re

    html = html_path.read_text(encoding="utf-8")
    base = html_path.resolve().parent

    def inline(m: "re.Match") -> str:
        tag = m.group(0)
        if "stylesheet" not in tag:
            return tag
        href = re.search(r'href=["\']([^"\']+)["\']', tag)
        if not href:
            return tag
        css = base / href.group(1)
        if not css.exists():
            return tag
        text = css.read_text(encoding="utf-8")
        # An inlined <style> is terminated by the FIRST "</style>" the HTML
        # parser sees — even inside a CSS comment. Neutralize any such literal
        # so the inlined stylesheet can't close itself early and break the page.
        text = re.sub(r"</\s*style", r"<\\/style", text, flags=re.I)
        return f"<style>\n{text}\n</style>"

    html = re.sub(r"<link\b[^>]*>", inline, html)
    dest = out_path.with_suffix(".source.html")
    dest.write_text(html, encoding="utf-8")
    print(f"kept self-contained source HTML → {dest}")


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description="Convert HTML slides to an editable .pptx.")
    ap.add_argument("html", type=Path, help="HTML file with one or more .slide elements")
    ap.add_argument("-o", "--out", type=Path, required=True, help="output .pptx path")
    ap.add_argument("--work", type=Path, default=None, help="dir for intermediate PNGs")
    ap.add_argument("--scale", type=int, default=2, help="device scale factor (default 2)")
    ap.add_argument("--dump-json", type=Path, default=None, help="write scraped slide JSON (debug)")
    ap.add_argument("--keep-html", action=argparse.BooleanOptionalAction, default=True,
                    help="save a self-contained copy of the source HTML (linked CSS "
                         "inlined) next to the output as <out>.source.html "
                         "(default: on; pass --no-keep-html to skip)")
    args = ap.parse_args(argv)

    if not args.html.exists():
        print(f"no such file: {args.html}", file=sys.stderr)
        return 2

    tmp = None
    work = args.work
    if work is None:
        tmp = tempfile.TemporaryDirectory()
        work = Path(tmp.name)
    work.mkdir(parents=True, exist_ok=True)

    try:
        slides = render_and_scrape(args.html, work, args.scale)
        if args.dump_json:
            args.dump_json.write_text(json.dumps(slides, indent=2, ensure_ascii=False),
                                      encoding="utf-8")
        build_pptx(slides, args.out)
        if args.keep_html:
            _export_source_html(args.html, args.out)
    finally:
        if tmp is not None:
            tmp.cleanup()

    n_text = sum(len(s["texts"]) for s in slides)
    n_shape = sum(len(s["shapes"]) for s in slides)
    n_pic = sum(len([p for p in s["pics"] if p.get("png")]) for s in slides)
    print(f"wrote {args.out} — {len(slides)} slides, {n_text} text boxes, "
          f"{n_shape} shapes, {n_pic} pictures")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
