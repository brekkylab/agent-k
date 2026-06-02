---
name: pptx
description: Guidelines for producing a slide deck (.pptx) that looks designed, not assembled. One palette, one type scale, one shape vocabulary, one grid — across the whole deck. A text-only slide is a failed slide. Read fully before producing any slide.
---

# PPTX Design Guidelines

Two non-negotiable rules:

1. **Every slide carries shapes that do work** — not decoration.
   Text-only slides fail. Always add at least one of: edge strip,
   custom bullet markers, hairline divider, corner block, sidebar
   tag, card grouping.
2. **Every slide obeys the same system** — same palette, two fonts,
   four type sizes, margins, shape vocabulary. If two slides pinned
   side by side show a different stroke weight or type size, the
   deck has failed.

**Hard rules — if any of these slip, the deck fails:**

- Write `outline.md` **before** any python-pptx code (Process §1)
- Verify gate: `verify()` returns empty AND every PNG read with
  the `read` tool — both required (Process §3)
- Charts: native `add_chart()` (editable in PowerPoint). Set EA
  typeface on every axis / legend / data label AND explicit fill
  color on every series (Rendering notes — Charts)
- No `MSO_AUTO_SIZE` on titles / KPI / tags / captions (Anti-patterns)
- Only `.pptx` surfaces to `artifacts/` (Anti-patterns)

---

## Process — outline first, then transcribe to pptx

Two phases. **Separate content from layout** — write everything as
markdown first, then transcribe into python-pptx.

### 1. Outline (`outline.md`) — verbatim slide content

**Do not skip this step** — Step 2's verbatim transcribe rule needs
an outline file to copy *from*; without it, content drifts back to
free-improvisation.

The outline contains the *actual content* that will appear on each
slide. It's not a plan or summary — it's the source of truth. One
section per slide, **including both bookends** — a "10 slides"
request means 1 title + 8 content + 1 closing, *not* 10 content
slides. Even when the prompt doesn't mention them, generate them.
Slide format:

**Slide 1 — title slide (always)**:
- `## Slide 1: <deck title>` — e.g. `## Slide 1: Q1 2026 Business Review`
- `**Eyebrow**` sub-line (caption-tier label, e.g. `**Pixellife Inc.**`)
- `**Subtitle**` sub-line (one-line framing, e.g. `**For the exec meeting**`)
- No bullets / tables / charts.

**Slides 2 to N−1 — content slides**:
- `## Slide N: <takeaway-headline>` — headline is a *takeaway
  sentence* (e.g. "Revenue grew 31.7%, margin improved in step"),
  not the topic (e.g. "Q1 results"). Becomes the slide title.
- A **bold sub-line** for the kicker / framing label
  (e.g. `**Executive Summary**`, `**Revenue by platform**`).
- Data as a **markdown table** — rendered on the slide as a styled
  table, KPI cards, or chart depending on the chosen layout.
- Narrative points as a **bulleted list** — rendered with the deck's
  bullet markers.
- Explicit chart / visual hints inline
  (e.g. `**Donut chart: revenue share by title**`) — rendered as
  a native python-pptx chart (editable in PowerPoint).

**Slide N — closing slide (always)**:
- `## Slide N: <closing message>` — e.g.
  `## Slide 10: Thank you. — Questions?`
- No bullets / tables / charts (Action-plan variant exception aside).

A 10-slide outline runs ~200–400 lines. Save as `outline.md` in the
working directory (not under `artifacts/` — see Anti-patterns).
Delete when done if turns share state.

### 2. Transcribe outline → pptx

Read each `outline.md` section and emit one slide. **Transcribe text
verbatim — don't rewrite, paraphrase, or shorten.** Pick the layout
grammar from the section's content shape (KPI showcase for KPI
tables, card grid for parallel items, asymmetric split for chart +
sidebar) and vary across the deck. Insert charts as native
python-pptx chart objects (`add_chart`) with explicit CJK fonts and
series colors (Rendering notes — Charts). Apply the locked palette,
type scale, margins, and footer to every slide — consistency lives
here, not in the content.

### 3. Verify the output renders correctly

**Gate: do not surface the `.pptx` until BOTH (A) `verify()` returns
all empty lists AND (B) you have rendered to PNG and read every
slide. Step B is mandatory regardless of whether Step A flagged
anything — they catch different failures.**

**A. Run `verify_pptx.py`** — the skill ships a compliance checker:

```python
import sys; sys.path.insert(0, "/workspace/skills/pptx/script")
from verify_pptx import verify, summarize

issues = verify("/workspace/artifacts/deck.pptx")
print(summarize(issues))
```

It returns a dict with five checks: `palette` (gallery match + drift),
`fonts` (missing East-Asian typeface), `sizes` (5+ scale sizes on a
slide), `overflow` (boxes whose CJK-aware text estimate exceeds box
height, including autosize-grow that may overlap neighbors), and
`page_numbers` (title / closing carrying `n / N`). An empty list per
key = passed. Use the *slide indices* in non-empty lists to target
fixes.

**Do not `read` or `cat` `verify_pptx.py`** unless you genuinely need
to debug an unexpected exception from the helper. The function names
and behaviour above are the full contract — reading the source file
just burns context.

**B. Visual confirmation** — `verify_pptx` is heuristic. After fixing
its flagged issues, render to PDF + PNG and look. **The image is
pre-baked with `soffice` + LibreOffice but NOT with `pdftoppm`** —
install `poppler-utils` first, every time, before the PNG step:

```
apt-get install -y poppler-utils       # provides pdftoppm — not in image
soffice --headless --convert-to pdf out.pptx
pdftoppm -r 100 -png out.pdf slide
```

**Open each PNG with the `read` tool — this loads the image into
your multimodal vision so you actually *see* the rendered slide.**
`ls`, `file`, `cat`, or any text-mode operation on a PNG is NOT
inspection — confirming the file exists is not the check. Make one
`read` call per slide PNG (`slide-1.png`, `slide-2.png`, …) and
check every slide for **correctness**:

- CJK tofu (`□□□`) in body, chart axes, legend, and data labels
  (most common cause: missing EA typeface on chart elements)
- Default PowerPoint chart colors (blue / orange / gray) leaking
  through because a series was created without explicit fill
- **Chart bbox overruns into narrative / insight / bullet text below
  or beside it** (compute `insight_y` from `chart_y + chart_h + GUTTER`)
- Overlap cases that the verify heuristic only *suspects*
- Shape position drift / misalignment against the grid
- Text boxes pushed out of place (clipping, shifted off-canvas)
- Alignment imbalance across the deck (same role element sitting
  at different x/y across consecutive slides)

…and for **design quality** (`verify_pptx` cannot catch these):

- Slide 1 IS a title slide pattern (anchor + three-tier text +
  signature mark), not a content slide labeled "Title"
- The last slide IS a closing slide pattern (mirrors the title)
- Layouts vary across the deck — not every content slide using the
  same division (e.g. 8 card-grids in a row = failure)
- Palette fits the deck's subject / mood — not Corporate Slate by
  default whenever the brief sounds "business"
- Slide titles are takeaway sentences, not topic labels
  ("Revenue grew 31.7%" ✓ / "Q1 Results" ✗)

**Fix strategy depends on issue type**:

- **Correctness** (tofu, overlap, drift, clipping, page numbers on
  bookends) — apply slide-level surgery directly to the `.pptx`:

  ```python
  from pptx import Presentation
  from pptx.util import Inches, Pt
  prs = Presentation("out.pptx")
  slide = prs.slides[4]                          # slide index = N − 1
  shape = slide.shapes[7]

  # move (left, top) + resize (width, height)
  shape.left,  shape.top    = Inches(0.8), Inches(1.8)
  shape.width, shape.height = Inches(5.2), Inches(0.8)

  # patch a text run (shorten / retype / fix font)
  run = shape.text_frame.paragraphs[0].runs[0]
  run.text      = "..."
  run.font.size = Pt(14)

  # remove a shape (e.g. stray page number on a bookend slide)
  bad = slide.shapes[9]._element
  bad.getparent().remove(bad)

  prs.save("out.pptx")
  ```

  Then re-render PNG and re-verify — no full re-transcribe.

- **Design quality** (bookend missing, layout monotony, palette
  mismatch, topic-not-takeaway titles) — return to Step 1 (outline)
  or Step 2 (transcribe), fix at the source, re-emit the deck.

Loop until both `verify()` and PNG inspection pass — partial pass is
failure.

---

## Palette — color roles

The *role structure* is fixed. The *hex values* are chosen per deck
(matched to subject and mood) and frozen for that deck's lifetime.

**Colored fills** — Primary (anchor; slide titles, deepest text),
Secondary (principal accent; left edge strip, KPI top strips, 1st
chart series, category tags), Accent (warm pop; eyebrows, corner
blocks, quote strip, "current" timeline marker — use sparingly).

**Surfaces** — Background (page; near-white or near-black/navy),
Surface (card / tile fill; subtly lifts off Background), Tinted
Panel (Surface ~5–10% toward Accent; for callout / insight bands).

**Structural** — Muted (supporting text: subtitles, captions,
footers, axis labels), Hairline (structural lines only: dividers,
footer line, card borders, timeline tracks).

**Status colors** — Positive (green, `+18%`), Negative (red, `-3d`).
Deltas only, never a fill.

**Rule of three.** Per slide, ≤ 3 colored fills from
{Primary, Secondary, Accent}. A fourth turns it into a paint chart.

---

## Palette gallery — choose by feel, never default

At the start of every deck:

1. Read the title and subject. What mood? (Authoritative? Cinematic?
   Editorial? Playful? Academic?)
2. Pick the palette from below whose feel matches — or compose a new
   one in the same style.
3. Lock those nine hex values. Never change them mid-deck.

**Match the palette to *this* deck's mood, not to deck history.** If
you reach for Corporate Slate by default whenever the brief mentions
"business" or "strategy," you are anchoring — re-read the deck's
subject and pick the palette whose feel actually fits.

**Prefer tonal / pastel siblings over pure-saturation primaries.**
Pure `#0000FF` / `#FF0000` / `#FFFF00` read as PowerPoint defaults.
The gallery palettes already use tonal-down hues (`#2563EB` not
`#0000FF`; `#F59E0B` not `#FFC107`); keep this when composing new.

---

**Corporate Slate** — *restrained, authoritative, financial.* Strategy, board updates, consulting, finance.
```
Primary    #0F172A   Secondary #2563EB   Accent   #F59E0B
Background #F8FAFC   Surface   #FFFFFF   Muted    #64748B
Hairline   #E2E8F0   Positive  #10B981   Negative #EF4444
```

**Midnight Keynote** — *cinematic, bold, on-stage.* Product launch, high-energy pitch; Primary and Background collapse to deep dark.
```
Primary    #0B1220   Secondary #818CF8   Accent   #22D3EE
Background #0B1220   Surface   #1F2937   Muted    #94A3B8
Hairline   #1F2937   Positive  #34D399   Negative #F87171
```

**Warm Editorial** — *magazine-like, narrative, hospitable.* Brand, storytelling, retail / hospitality / lifestyle.
```
Primary    #7C2D12   Secondary #EA580C   Accent   #FACC15
Background #FFF7ED   Surface   #FFFFFF   Muted    #9A3412
Hairline   #FED7AA   Positive  #15803D   Negative #B91C1C
```

**Forest Research** — *calm, considered, exact.* Academic, R&D, white papers; green Secondary signals "verified" without corporate blue.
```
Primary    #1F2937   Secondary #047857   Accent   #D97706
Background #FFFFFF   Surface   #F9FAFB   Muted    #6B7280
Hairline   #E5E7EB   Positive  #059669   Negative #DC2626
```

**Mono Editorial** — *typographic, silent, luxury.* Manifesto, book chapter, design portfolio; emphasis moves to weight & italic.
```
Primary    #111827   Secondary #111827   Accent   #111827
Background #FFFFFF   Surface   #FFFFFF   Muted    #6B7280
Hairline   #E5E7EB   Positive  #059669   Negative #DC2626
```

**Playful Violet** — *lively, contemporary, consumer.* App launch, community / event, casual brand.
```
Primary    #4C1D95   Secondary #14B8A6   Accent   #F472B6
Background #FAF5FF   Surface   #FFFFFF   Muted    #6D28D9
Hairline   #E9D5FF   Positive  #16A34A   Negative #E11D48
```

**Sand & Ink** — *quiet, journalistic, considered.* Op-eds, foundation reports, slow-brand; warm neutrals + single deep ink.
```
Primary    #1C1917   Secondary #57534E   Accent   #B45309
Background #FAFAF9   Surface   #FFFFFF   Muted    #78716C
Hairline   #E7E5E4   Positive  #15803D   Negative #B91C1C
```

**Glacier** — *clean, scientific, optimistic.* Healthcare, climate-tech, data products that want modern without corporate.
```
Primary    #0E7490   Secondary #0891B2   Accent   #F97316
Background #F0F9FF   Surface   #FFFFFF   Muted    #475569
Hairline   #BAE6FD   Positive  #16A34A   Negative #DC2626
```

If none fits, compose a new palette in the same style — keep the
role distribution, change the hue family.

---

## Typography

One title font, one body font, picked once per deck and never mixed
with a third. Pick faces that render identically on every machine the
deck might open on:

- **Latin-only decks** — Calibri, Arial, or Helvetica
- **CJK / mixed-script decks** — use `Noto Sans KR` (Korean) /
  `JP` (Japanese) / `SC` (Simplified Chinese) / `TC` (Traditional
  Chinese). These are the Google Fonts family names; Google Slides
  resolves them natively, the sandbox's `fonts-noto-cjk` package
  registers them via fontconfig aliases, and PowerPoint / Keynote
  fall back to the host's system Korean / Japanese font when the
  exact name isn't present. See Rendering notes for setup.

| Size | Pt | Weight | Use |
|---|---|---|---|
| **Display** | 54 | Bold | Title slide, closing slide, large quotes |
| **Title** | 32 | Bold | Slide titles, section divider titles |
| **Body** | 18 | Regular | Bullets, paragraphs, KPI labels, narrative |
| **Caption** | 11 | Bold UPPERCASE | Eyebrows, tags, footers, page numbers, axis labels, attribution |

Line height: titles 1.05, body 1.15. Letter-spacing on caption-tier
only.

**Dramatic-beat sizes** sit *outside* the four-step scale and are
allowed on top of it — they're scenography, not text hierarchy. Use
sparingly, one per slide where the pattern calls for it: section
divider numerals (3–4× Title), quote-slide glyph (~4× Display),
Number Cover hero (180–250 pt). These don't count against the
"keep the scale tight" discipline.

**Korean / CJK refinement.** Hangul fonts default to looser tracking
and line-height than Latin — untuned Korean reads "floaty." For
Korean runs: letter-spacing −1% on body, −2 to −3% on titles;
line-height on multi-line titles 1.0–1.1 (tighter than Latin 1.05) so
two-line Korean titles read as one visual unit.

**One emphasis per slide** — bold *or* color, not both, one phrase
max. Three emphasized things means it's actually three slides.

---

## Grid (16:9, 13.333" × 7.5")

All absolute dimensions below assume this canvas. For other slide
sizes (4:3, 9:16, custom widths), scale *proportionally* — keep the
same ratios of margin to content, title band to body, footer offset
to bottom edge. The relationships are the system; the literal inches
are just calibrated for the most common case.

- Side margins 0.6" L/R, top 0.5", bottom 0.45"
- **Title band** — top 1.1" of every content slide. Eyebrows above,
  title inside. Body content starts at y ≈ 1.6"
- **Footer** at y ≈ 7.1" — hairline divider + page number `n / N`
  right (caption muted). Optional short deck title left. **Content
  slides only** — title, section dividers, and closing slide carry
  no footer / page number.
- **Content area** — 12.13" × 5.0"

Title slide, section dividers, quote slide, and closing slide
intentionally break the grid — those are the dramatic beats.

**Asymmetric margins by intent.** Equal margins on all four sides
read "safely centered" — fine for academic decks, monotonous
elsewhere. Pick *one* axis of asymmetry per deck (wider-left + tighter-
right, or wider-bottom + tighter-top, etc.) and apply it consistently.
Don't randomize per slide.

---

## Rendering notes — correctness, not style

These aren't design choices; they're things python-pptx leaves to you
that quietly ruin a deck if missed.

### CJK text — set Latin + EA typeface on every run

`font.name` only sets the Latin typeface; the East-Asian face
(`<a:ea>` in the run XML) stays unset and the viewer falls back to
whatever it has. For every CJK-containing run, set both Latin and
EA typefaces in a helper. The same applies to *chart text
elements* — see Charts section below.

`fonts-noto-cjk` ships in the sandbox image. For the pptx typeface
string, use the Google Fonts family name (NOT the TTC family name
with the "CJK" infix): `'Noto Sans KR'`, `'Noto Sans JP'`,
`'Noto Sans SC'`, or `'Noto Sans TC'`.

### Charts — native python-pptx for editability

Charts are rendered as **native python-pptx chart objects**
(`slide.shapes.add_chart(...)`) so the user can edit chart data,
labels, and colors directly in PowerPoint (right-click → Edit Data).
The trade-off: python-pptx leaves two critical things unhandled that
you MUST set yourself, every time.

**1. CJK font on every chart text element**

`font.name` only sets Latin. Without explicit East-Asian typeface,
PowerPoint falls back to whatever the opener has — typically tofu
on Korean / Japanese / Chinese axes and labels. Apply EA typeface
to every text element on the chart: category axis, value axis,
data labels, legend, chart title.

```python
from pptx.oxml.ns import qn
from pptx.oxml.xmlchemy import OxmlElement
from pptx.util import Pt
from pptx.dml.color import RGBColor

def set_chart_text_font(font, typeface='Noto Sans KR', size_pt=11):
    """Set Latin + EA + CS typefaces on a chart font, with size."""
    font.name = typeface
    font.size = Pt(size_pt)
    rPr = font._element.get_or_add_rPr()
    for tag in ('latin', 'ea', 'cs'):
        el = rPr.find(qn(f'a:{tag}'))
        if el is None:
            el = OxmlElement(f'a:{tag}')
            rPr.append(el)
        el.set('typeface', typeface)

for axis in (chart.category_axis, chart.value_axis):
    set_chart_text_font(axis.tick_labels.font, size_pt=11)
if chart.has_legend:
    set_chart_text_font(chart.legend.font, size_pt=11)
if chart.has_title:
    set_chart_text_font(
        chart.chart_title.text_frame.paragraphs[0].runs[0].font, size_pt=14)
for ser in chart.series:
    if ser.data_labels:
        set_chart_text_font(ser.data_labels.font, size_pt=11)
```

**2. Series colors must be explicit — every series, every time**

Without explicit fill, python-pptx falls back to PowerPoint theme
colors (default blue / orange / gray) — leaks regardless of the
deck's chosen palette. Set every series:

```python
PALETTE_ORDER = ['Secondary', 'Accent', 'Primary']  # max 3 series
for ser, role in zip(chart.series, PALETTE_ORDER):
    hex_ = palette[role]
    ser.format.fill.solid()
    ser.format.fill.fore_color.rgb = RGBColor.from_string(hex_)
    ser.format.line.color.rgb = RGBColor.from_string(hex_)
```

**Bbox + adjacency math — chart bbox and narrative bbox computed**

```python
from pptx.enum.chart import XL_CHART_TYPE
from pptx.chart.data import CategoryChartData

chart_x, chart_y, chart_w, chart_h = 0.6, 1.7, 7.5, 3.0
GUTTER = 0.3
insight_y = chart_y + chart_h + GUTTER   # adjacency math, computed not eyeballed

data = CategoryChartData()
data.categories = ['Q1', 'Q2', 'Q3', 'Q4']
data.add_series('2025', (142, 158, 165, 175))
data.add_series('2026', (187, 0, 0, 0))

chart = slide.shapes.add_chart(
    XL_CHART_TYPE.COLUMN_CLUSTERED,
    Inches(chart_x), Inches(chart_y),
    Inches(chart_w), Inches(chart_h),
    data,
).chart
# … then apply CJK font helper + series colors above …
```

**Supported chart types** — Bar (clustered / stacked), Line, Pie,
Doughnut, Area, Scatter, Combo (bar + line). For data that doesn't
fit one of these (Treemap, Waterfall, Sunburst patterns) — convert
to a styled table or decompose into a simpler chart type. Don't
mix matplotlib PNGs in: it breaks the editability promise.

**Data label format** — use PowerPoint number-format masks:
```python
ser.data_labels.show_value = True
ser.data_labels.number_format = '#,##0"억 원"'    # → 187억 원
# Or '0.0%' for percentages, '+#,##0;-#,##0' for signed deltas.
```

### Text-box sizing — fixed-height boxes, no autosize for short elements

python-pptx doesn't render text; an undersized box silently clips
when PowerPoint opens. **Design boxes generously up front; do not
lean on autosize.** `MSO_AUTO_SIZE.SHAPE_TO_FIT_TEXT` is a trap on
short fixed-shape elements — autogrowth overlaps neighbors and
breaks the grid.

Korean / CJK glyphs occupy ~2× ASCII width: a 2.5" box holds ~14
Korean chars per line at 14 pt body, *not* 30. And CJK wraps where
the estimator predicts one line. **For CJK decks, assume every
content-slide title is 2 lines** — size title bands accordingly, or
step title size 32 → 26–28 pt to fit one line reliably.

Rules by element:
- **Titles, KPI values, tags, chips, captions** — fixed height, no
  autosize. Tag chips ≥ 0.45" tall; KPI cards ≥ 1.5". When the box
  is taller than the text inside (KPI cards especially), set
  `text_frame.vertical_anchor = MSO_ANCHOR.MIDDLE` — the default
  (TOP) glues text to the top edge and leaves dead whitespace below.
- **Bullets / body paragraphs** — size to `(line_count + 1) *
  line_height` minimum (use `box_height_in()` / `line_count()` from
  `verify_pptx.py`). Autosize allowed *only here*, only with
  verified neighbor clearance.
- **Hero text** (Display, Number Cover, quote glyph) — text drives
  the box; size the box to the text.

Both `verify_pptx` overflow checks (estimated and autosize-grow)
must return empty before delivery.

### Other pitfalls
Every shape inherits a default shadow on creation — set
`shape.shadow.inherit = False` on every shape or the banned effects
sneak back. Layer order: place full-slide background, anchor blocks,
edge strips, and card fills *before* any text or the text disappears
under shapes drawn after it.

---

## Slide patterns

Pick by the slide's *job*, not by what looks pretty. Each pattern is
a starting point — compose freshly within it, don't trace the same
layout every deck.

- **Title slide (slide 1)** — deck's visual signature. Anchor shape +
  three-tier text (eyebrow caption-Accent → Display-Primary title →
  Body-Muted subtitle) + a small Accent signature mark. **Compose
  freshly each time**; defaulting to "filled rectangle in one corner"
  is the AI tell. No page number.
- **Section divider** — full-bleed Primary BG, huge numeral ("01") at
  3–4× Title size in lightened Secondary at left, section title at
  right (Display, white), Accent strip beneath. No page number.
- **Content slide** — left edge strip + title band + bulleted body.
  **4–6 bullets max** (cap at 3–4 for CJK / Korean). Title = takeaway
  sentence, not topic.
- **Two-column comparison** — vertical hairline split, Category tag
  atop each column (Secondary left, Accent right), 3–5 bullets each.
  Keep left/right meaning consistent across all comparison slides.
  *Wins / concerns variant:* drop tags, swap markers for ✓ / ✕.
- **KPI showcase** — 2–4 cards, 0.3" gutters. Each: Surface fill, no
  shadow, 0.5pt Hairline border, 0.12" Secondary top strip, Display
  value (44 pt if 4 cards) + caption-uppercase label (Muted) +
  optional delta. Pick top-strip or left-strip orientation per deck.
- **Quote slide** — oversized `"` glyph (≈4× Display, Accent toward
  Background), quote at Title size ≤ 4 lines, attribution (Body,
  Muted) right-aligned below a short Accent strip.
- **Chart slide** — title = takeaway ("Paid users overtook free in
  Q2"), not subject ("MAU by tier"). Native `add_chart` (see
  Rendering notes — Charts); series palette = Secondary, Accent,
  Primary in that order; **max 3 series** (4th → use a table). Set
  EA typeface on every chart text element AND explicit fill on every
  series — otherwise CJK tofu / default theme color leak. Reserve
  the chart bbox and compute `insight_y` from `chart_y + chart_h +
  GUTTER` to avoid overrun.
- **Timeline** — horizontal hairline track + 3–6 evenly-spaced event
  markers. Period label (caption, Accent) above, milestone label
  (Body bold, Primary) + optional caption-Muted detail below. One
  marker may swap to Accent for "current."
- **Closing slide (slide N)** — mirrors the title composition (same
  palette, same shapes, position flipped — opposite corner / panel /
  quadrant). Display message ("Thank you.", "Questions?") in the
  title's position-equivalent. No page number. *Action-plan variant:*
  dark Primary BG, numbered action cards left + risk bullets right +
  bottom Accent-tinted "Decisions needed" strip.

---

## Anti-patterns — refuse to ship

- Text-only slide (no shapes doing work)
- Default bullet characters (`●` `-` `>` `■`)
- Third font, or 5+ sizes *from the four-step scale* on one slide
  (dramatic-beat scenography sizes don't count)
- Centered body text (only KPI values and divider numerals center)
- 7+ bullets on a content slide
- Drop shadows, glow, gradients, 3D, clipart, SmartArt, curved
  decorative lines, default PowerPoint placeholders
- Default PowerPoint chart colors
- Page number on title / divider / closing slides
- Two patterns merged into one slide
- New hex color introduced mid-deck
- Relying on `MSO_AUTO_SIZE` for titles, KPI values, tags, captions,
  or any short fixed-shape element (autogrowth overlaps neighbors)
- Native `add_chart()` without explicit EA typeface on every chart
  text element (axes / legend / data labels / title) — silent CJK
  tofu in PowerPoint
- Native `add_chart()` without explicit `fill.fore_color.rgb` on
  every series — PowerPoint theme defaults (blue / orange / gray)
  leak through and break the deck palette
- Multiple files in `artifacts/` — only the final `.pptx` belongs
  there. Chart PNGs, verify PDFs/PNGs, helper scripts, and
  `outline.md` all stay in the working directory.

---

A quiet, consistent deck reads as designed. A loud, varied deck reads
as assembled.