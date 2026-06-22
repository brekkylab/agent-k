---
name: pptx
description: Produce a slide deck (.pptx) that looks designed, not assembled, by authoring each slide in HTML/CSS and converting to an editable PPTX. One palette, one type scale, one shape vocabulary, one grid — across the whole deck. A text-only slide is a failed slide. Read fully before producing any slide.
---

# PPTX Design Guidelines (HTML-authored)

You author slides as **HTML + CSS** (charts via Chart.js / SVG / images,
freely), then run `html2pptx.py` to convert them to an **editable
`.pptx`**. The converter **decomposes** each slide into native objects
rather than one flat image:

- **Text** → native, editable text boxes.
- **Simple shapes** (solid-fill boxes, borders, hairline dividers, edge
  strips, card top-strips, bullet markers) → native autoshapes you can
  move and recolor.
- **Charts / images / SVG** (Chart.js `<canvas>`, `<img>`, inline
  `<svg>`) → each its **own picture object** (separate, movable) — their
  internal text/data is raster, not editable.
- **Page backdrop** → the slide's solid background color becomes a
  native slide fill (no full-slide image). Only un-modelable content
  (gradient / shadow / background-image) falls back to a base picture.

So text and simple shapes round-trip as real, editable PowerPoint
objects; only chart/image internals stay raster. Stacking follows
document order. Design fidelity is exactly what Chromium renders.

Two non-negotiable rules:

1. **Every slide carries shapes that do work** — not decoration.
   Text-only slides fail. Always add at least one of: edge strip,
   custom bullet markers, hairline divider, corner block, sidebar
   tag, card grouping. `components.css` ships these.
2. **Every slide obeys the same system** — same palette, two fonts,
   four type sizes, margins, shape vocabulary. If two slides pinned
   side by side show a different stroke weight or type size, the
   deck has failed.

**Hard rules — if any of these slip, the deck fails:**

- Install the toolchain before converting (the sandbox ships neither).
  Run all three up front — **do not** wait for a launch failure to add
  `install-deps`; the sandbox is always missing the browser's system
  libraries, so skipping it just wastes a failed conversion:
  `pip install python-pptx playwright` then
  `playwright install chromium --only-shell` then
  `playwright install-deps chromium`.
- Each slide is a `<div class="slide">` sized **exactly 1280×720 px**
  (16:9). Author at 96 px/in. **No `transform: scale()` / `zoom`** on
  slide content — it breaks the px→inch mapping.
- CJK decks: set `lang` on `<html>` and use the **exact** family names
  `'Noto Sans CJK KR'` / `JP` / `SC` / `TC` (the `CJK` infix is
  mandatory — `'Noto Sans KR'` falls back to tofu).
- Verify gate: `verify()` reports no issues AND the contact-sheet
  overview `read` (+ any suspect slide deep-read) — both required (§3).
- Only the final `.pptx` (plus the `.source.html` sidecar the converter
  auto-saves beside it) belong in `artifacts/` — your working `deck.html`
  and intermediate PNGs stay in the working directory.

---

## Process — HTML → convert → verify

### 1. Author the HTML

Work straight from the source material into `deck.html` — no separate
outline file. Plan the deck first: **include both bookends** — a "10 slides"
request means 1 title + 8 content + 1 closing, *not* 10 content slides;
generate the bookends even when the prompt doesn't mention them. Give every
content slide a **takeaway-sentence** title ("Revenue grew 31.7%, margin
improved in step"), not a topic label ("Q1 results"). Title slide: deck title
+ eyebrow + one-line subtitle, no bullets/tables/charts. Closing slide: a
closing message, no bullets/tables/charts (Action-plan variant aside).

Create one `deck.html` linking `components.css` (copy it from the skill
`script/` dir next to this file, or `<link>` it by path). At the top of
the file, **define this deck's composed palette** — a `<style>` block (or
`:root`) setting the nine role variables you derived in the Palette step. Set
`<html lang="ko">` for Korean. Emit one `<div class="slide">` per slide,
keeping text **verbatim** from the source — don't rewrite or shorten. Data as
styled tables, narrative as bulleted lists. Pick the layout grammar from each
slide's content shape (styled table or compact stat row for metrics, card grid
for parallel items, asymmetric split for chart + sidebar) and vary across the deck.

`script/template.html` is a **mechanics** reference (how to define the
palette variables, structure a `.slide`, use the components, and signal
Chart.js readiness) — **not a look to copy**. Read it once for the wiring,
then compose your own palette and vary the layouts for your subject; don't
reproduce its colors or slide order, or every deck ends up looking the same.

**Charts carry no editability requirement** — each becomes its own raster
picture object (Chart.js `<canvas>`, inline SVG, or static `<img>`). For the
Chart.js settings (`animation:false`, palette colors, `font.family`,
`__pptx_ready`) see *Authoring notes → Charts* and `template.html`.

### 2. Convert to PPTX

```
python /workspace/skills/pptx/script/html2pptx.py deck.html \
    -o /workspace/artifacts/deck.pptx
```

The converter renders each `.slide` in headless Chromium and emits a
native object per element — text boxes (Latin **and** EA typeface on
every run), autoshapes for simple boxes/borders/markers, a picture per
chart/image, and a solid slide-background fill. It also auto-saves a
self-contained copy of the source HTML (linked CSS inlined) next to the
`.pptx` as `<out>.source.html` for review (pass `--no-keep-html` to skip;
`--dump-json out.json` to inspect the scraped geometry).

### 3. Verify the output renders correctly

**Gate: do not surface the `.pptx` until BOTH (A) every `verify()` check
is clean AND (B) the contact sheet has been `read`-inspected (plus any
suspect slide deep-read) and every flagged issue fixed.** B is mandatory
even if A is clean — they catch different failures.

**A. Run `verify_pptx.py`:**

```python
import sys; sys.path.insert(0, "/workspace/skills/pptx/script")
from verify_pptx import verify, summarize
issues = verify("/workspace/artifacts/deck.pptx")
print(summarize(issues))
```

Five checks, on the emitted text boxes:

- `fonts` — CJK runs with no East-Asian typeface (tofu risk)
- `sizes` — 5+ distinct tier / out-of-tier sizes on one slide
- `page_numbers` — title (1) or closing (N) slide carrying `n / N`
- `overlap` — text boxes whose bboxes collide (layered/stacked pairs
  filtered out)
- `palette` — discipline: flags palette **sprawl** (too many distinct
  non-neutral colors) and reports any color used on a single slide;
  advisory — lean on B for whether the palette fits the subject

`fonts`/`sizes`/`page_numbers`/`overlap` pass when their lists are empty.
(No `overflow`/`word_wrap` check — boxes are sized to browser-measured
text and line breaks are frozen, so clipping/re-wrap can't happen; no
chart check — charts are raster.) **Do not `read` `verify_pptx.py`** —
these checks are the full contract; only read the source on an
unexpected exception.

**B. Visual confirmation** — render to PDF + PNG, montage into ONE contact
sheet, and look. Install `poppler-utils` first (provides `pdftoppm`):

```
apt-get install -y poppler-utils
soffice --headless --convert-to pdf /workspace/artifacts/deck.pptx
pdftoppm -r 75 -png deck.pdf slide
python /workspace/skills/pptx/script/contact_sheet.py slide*.png -o contact.png
```

**`read` `contact.png` ONCE** for a whole-deck overview, then deep-`read`
only the individual `slide-N.png` that look off (or that verify-A / a
converter `WARNING` flagged). Reading every slide individually is the
biggest token cost and is unnecessary — the contact sheet shows tofu,
empty space, misalignment, off-canvas cut-offs, and chart-color problems
across all slides at once; zoom in only where needed. Check for
**correctness**:

- CJK tofu (`□□□`) anywhere (most common: a text run or chart label
  whose font family didn't resolve)
- **Text box misplaced** — a box sitting at the wrong x/y vs its HTML
  position (a slide looks shifted/misaligned)
- Text clipped or pushed off-canvas
- **Anything cut off at a slide edge.** If the converter prints
  `WARNING: slide N … overflowing the 1280x720 canvas`, a card/shape/text
  runs past the edge — fix the HTML so it fits (narrower columns, smaller
  gap, less padding). Right-column grids are the usual offender.
- Chart rendered blank or with wrong colors in the background
- Alignment imbalance (same-role element at different x/y across slides)

…and for **design quality** (`verify_pptx` cannot catch these):

- Slide 1 opens on something specific to THIS subject (not a generic
  anchor + three-tier-text template), and the deck's **signature** is
  present and recurring across slides
- The last slide IS a closing pattern (mirrors the title)
- Layouts vary — not every content slide using the same division
- **No large dead space** — content fills the canvas; the bottom third
  isn't blank and content isn't all stacked to one side (Grid → "Fill
  the canvas"). Tall containers around short text are the usual cause
- Palette is composed from the subject and actually shows its Accent — not
  the default navy/blue/amber-on-white reflex
- **Distinctiveness check** — would this exact palette + layout fit an
  unrelated subject just as well? If yes, it's a default: push the accent /
  signature toward something this subject justifies, and note what you changed
- Slide titles are takeaway sentences, not topic labels

**Fix at the source**: edit the HTML (position, size, font, content) and
re-run the converter — there is no slide-level pptx surgery step.

**Work in batches, not one pixel at a time.** Read all slides, list every
issue, fix them across the HTML, then re-render **once**. Aim for **≤ 2–3
visual passes total** — re-rendering after each tiny edit is the main
reason a deck takes forever. In particular, **do not hand-chase CJK
line-wrapping**: the converter **freezes the browser's exact line breaks**
(text boxes are `word_wrap`-off with hard `<a:br/>` breaks), so
soffice/PowerPoint render the same lines you saw in the browser — re-wrap
and overflow can't happen. Intervene only on real layout issues
(off-canvas overflow, element collisions, empty space).

Loop until both A and B pass — partial pass is failure.

---

## Start from the subject (do this first)

Before any visual choices, write one line each: **what this deck is about,
who's in the room, and the one thing it has to accomplish.** A Q1 review for
a gaming studio's board and an H1 retrospective for a fragrance brand should
not come out looking interchangeable. Pull the deck's look from the subject's
own world — its product, materials, vocabulary, mood. If memory holds the
user's context or past decks, treat it as a hint.

Decide two things up front and hold them across every slide:

- **A composed palette** (next section) — built for this subject, not lifted
  from a stock theme.
- **A signature** — the one element a viewer walks away remembering: a
  recurring motif or treatment drawn from the subject (a marker shape, a
  numeral style, a divider, how charts are framed). Put the deck's single
  visual risk here and keep the rest restrained — one signature, not a
  scatter of effects.

## Palette — compose per deck (no fixed gallery)

The *role structure* is fixed; the *hex values are composed for THIS deck*
and frozen for its lifetime. There is no gallery to pick from — derive one.

**Roles** — set as CSS variables in the deck's `<style>`; `components.css`
consumes them:
- **Primary** — anchor: titles, deepest text.
- **Secondary** — principal accent: edge strip, card strips, 1st chart
  series, category tags.
- **Accent** — one warm / contrasting pop: eyebrows, the signature, emphasis.
  Use sparingly, but **do use it** — an unused accent reads as unfinished.
- **Background** / **Surface** — page / card-tile fill.
- **Muted** — subtitles, captions, axis labels.
- **Hairline** — dividers, borders, tracks.
- **Positive** / **Negative** — green / red deltas only, never a fill.

**Compose like this:**
1. Pick a **hue family from the subject's world** (its key art, packaging,
   industry, mood) — not a reflex default.
2. Set **Background + Surface** as near-neutral members of that family (a
   warm off-white, a cool paper, a deliberate dark) — not plain `#FFFFFF`
   unless that's a real choice.
3. Choose **one Accent with genuine contrast** against the background.
4. Keep hues **tonal, not pure-saturation** (`#2563EB`, not `#0000FF`).
5. Confirm **text-on-background contrast** is comfortably readable.
6. **Lock the nine values** for the whole deck. Rule of three: ≤ 3 colored
   fills from {Primary, Secondary, Accent} per slide; a fourth turns it into
   a paint chart. Never introduce a new hex mid-deck.

**Avoid the templated look.** If your palette is the one you'd reach for on
*any* business deck (navy + blue + amber on near-white is the usual reflex),
that's anchoring — push the hue family and accent toward what the subject
actually justifies. `components.css` ships an inert neutral fallback and a
placeholder compose recipe (`<...>` slots) — fill the slots with hexes you
derived; don't ship the placeholders or copy a previous deck's palette.

---

## Typography

One title font, one body font, picked once and never mixed with a third.
Pick faces that render identically in the sandbox and on opener machines:

- **Latin-only decks** — Calibri, Arial, or Helvetica.
- **CJK / mixed-script decks** — `Noto Sans CJK KR/JP/SC/TC` (the family
  names `fonts-noto-cjk` registers). soffice-rendered PNGs match exactly;
  PowerPoint/Keynote fall back to the host's Korean font when the exact
  name isn't present. The EA typeface is set on every emitted run by the
  converter, so CJK survives the round-trip.

`components.css` encodes the four-step scale as `.t-display` / `.t-title`
/ `.t-body` / `.t-caption`:

| Class | Px | Pt | Weight | Use |
|---|---|---|---|---|
| `.t-display` | 54 | 40 | Bold | Title/closing slides, large quotes |
| `.t-title` | 32 | 24 | Bold | Slide titles, divider titles |
| `.t-body` | 18 | 13.5 | Regular | Bullets, paragraphs, body text |
| `.t-caption` | 11 | 8 | Bold UPPERCASE | Eyebrows, tags, footers, axis + stat labels |

(Px values are what you author; the converter scales px×0.75 → pt.)

**Dramatic-beat sizes** sit *outside* the scale and are allowed on top
of it — section-divider numerals (3–4× title), quote glyph (~4×
display), Number-Cover hero (180–250 px). One per slide, sparingly.

**Korean/CJK refinement.** Hangul defaults to loose tracking — untuned
Korean reads "floaty." `components.css` applies `letter-spacing −1%`
body / `−2%` titles and tighter line-height to `:lang(ko)`. **Assume
content-slide titles wrap to 2 lines in Korean** — leave room in the
title band, or step the title down to ~26–28 px.

**One emphasis per slide** — bold *or* color, not both, one phrase max.

---

## Grid (1280×720 px = 13.333"×7.5", 16:9)

`components.css` encodes these; use the helper classes rather than
re-deriving offsets.

- Side margins ~58 px L/R, 48 px top, 43 px bottom (`.pad`).
- **Title band** — `.band-title`: top ~106 px of every content slide
  (eyebrow above, title inside). Body starts y ≈ 154 px (`.body-area`).
- **Footer** (`.footer`) — hairline + page number `n / N` right (caption
  muted), optional deck title left. **Content slides only** — title,
  dividers, and closing carry no footer / page number.

Title slide, section dividers, quote slide, and closing slide
intentionally break the grid — those are the dramatic beats.

**Asymmetric margins by intent.** Equal margins on all four sides read
"safely centered" — fine for academic decks, monotonous elsewhere. Pick
*one* axis of asymmetry per deck and apply it consistently; don't
randomize per slide.

**Fill the canvas — the most common failure.** The body area is ~500 px
tall (y 154 → 658); short content top-aligned in it leaves a dead bottom
third. **That blank lower band is the #1 complaint — treat it as a
defect to fix in verify-B, not a style choice.** Use the whole 1280×720:

- **Never stretch a tall container around short content.** A `.card` or
  column that spans the full `.body-area` height but holds 3 short lines
  leaves a dead bottom. Either **size the container to its content**
  (let it be short) or **distribute/vertically-center** the content
  (`justify-content: space-between` / `center`) so it uses the height.
- Spread content across **both axes**. If the left column is full and
  the right is empty (or top full / bottom empty), rebalance: widen
  columns, add a supporting visual, enlarge type, or increase spacing.
- A two-column split should have **both columns roughly balanced** in
  height — don't put 6 bullets left and 2 right with the rest blank.
- Prefer fewer, larger, well-spaced elements over a tight cluster
  floating in a sea of white. Generous *balanced* whitespace is design;
  a blank lower half is a hole.

---

## Authoring notes — correctness, not style

These quietly ruin a deck if missed.

### Geometry — keep px→inch exact
Slides are 1280×720 px and the converter maps 96 px → 1 inch. Anything
that warps that mapping corrupts every position: **no** `transform`,
`zoom`, or `devicePixelRatio` tricks on slide content. Position with
`absolute`/flex/grid in plain px. Stack slides vertically in the
document; the converter screenshots each `.slide` independently.

### What becomes editable vs raster
The converter decomposes each slide (see the four bullets at the top):
- **Text** → editable boxes. Each *text block* (element with direct
  text) is one box; inline children (`<b>`, `<span>`) become separate
  runs, so keep per-run styling on the block or its inline children.
  **Line breaks are frozen** at the browser's positions (hard `<a:br/>`,
  auto-wrap off) so the pptx matches the browser exactly — text stays
  editable, though typing into a box reflows only within that box.
- **Simple shapes** → native autoshapes. An element counts as a simple
  shape when it has a solid `background-color` and/or solid `border`(s)
  and **no** `background-image`/gradient, `box-shadow`, or `opacity<1`.
  Borders map to lines (uniform border → the shape's outline; a single
  side like `border-top` → a thin bar, i.e. hairlines & card strips).
  Bullet markers written as absolutely-positioned `::before` pseudo-
  elements are synthesized into real autoshapes automatically.
- **Charts / images / SVG** → each its own picture. The slide's solid
  background becomes a native fill; only un-modelable content
  (gradient / shadow / `opacity<1`) falls back to a base picture.
- **SVG `<text>` / `<canvas>` text stays raster** — don't rely on chart
  labels being editable.
- Don't overlap text boxes you intend to stay separate. (Line breaks are
  frozen, so soffice won't re-wrap them; `verify_pptx` + visual `read`
  remain the safety net.)

### Charts — Chart.js / SVG (own raster picture)
Native chart editability is **not** required. Render charts however
looks best:
- **Chart.js** (`<canvas>`): set `animation:false`, `responsive:true`,
  `maintainAspectRatio:false` inside a sized container; pull series
  colors from the palette CSS variables; set `font.family` on ticks/legend
  to the deck's font (a Noto CJK family for CJK, else labels go tofu);
  signal `window.__pptx_ready = true` after paint. (See `template.html`.)
- **Inline SVG** or a **static `<img>`** also work and need no readiness
  flag (but still wait on `document.fonts.ready`, which the converter
  does).
Max 3 series for readability; a 4th → use a table. Reserve the chart's
box and keep narrative/insight text outside it.

### Don't fight the converter
- One `.slide` per slide; nothing outside `.slide` is captured.
- Hidden elements (`display:none`, `visibility:hidden`, `opacity:0`) are
  skipped — use them for notes you don't want in the deck.
- Fully-transparent text color is treated as "no text" — don't hide
  real content that way.

---

## Slide patterns

Pick by the slide's *job*, not by what looks pretty. Compose freshly
within each — don't trace the same layout every deck.

- **Title slide (slide 1)** — the deck's opening statement, led by something
  concrete from THIS subject (a number, phrase, motif, or visual from its
  world) plus the deck's signature, eyebrow, and title. The
  eyebrow→`.t-display`→subtitle three-tier stack is a fallback, not the goal;
  "filled rectangle in one corner + three lines of text" is the AI tell —
  build something the subject earns. No page number.
- **Section divider** — full-bleed Primary background, huge numeral
  ("01") at 3–4× title in lightened Secondary left, section title right
  (Display, Surface), Accent strip beneath. No page number.
- **Content slide** — `.edge-strip` + `.band-title` + `.bullets` body.
  4–6 bullets max (3–4 for CJK). Title = takeaway sentence.
- **Two-column comparison** — vertical hairline split, category tag atop
  each column (Secondary left, Accent right), 3–5 bullets each. Keep
  left/right meaning consistent across all comparison slides.
- **Metrics** — present numbers as a **styled table** or a **compact
  inline stat row** (`.stat-row` → `.stat-value` + uppercase
  `.stat-label` + optional `.stat-delta`). Do **not** use big "KPI
  card" tiles — tall metric cards read as empty and monotonous. Vary
  the metric treatment across the deck.
- **Quote slide** — oversized `"` glyph (~4× Display, Accent toward
  Background), quote at Title size ≤ 4 lines, attribution (Body, Muted)
  right-aligned below a short Accent strip.
- **Chart slide** — title = takeaway ("Paid users overtook free in Q2"),
  not subject ("MAU by tier"). Chart in a sized container; series colors
  from the palette; reserve the box so narrative text never overlaps it.
- **Timeline** — horizontal hairline track + 3–6 evenly-spaced markers;
  period label (caption, Accent) above, milestone (Body bold, Primary) +
  caption-Muted detail below. One marker may swap to Accent for "current."
- **Closing slide (slide N)** — mirrors the title (same palette/shapes,
  position flipped to the opposite corner). Display message ("Thank
  you.", "Questions?") in the title's position-equivalent. No page
  number. *Action-plan variant:* dark Primary background, numbered action
  cards left + risk bullets right + bottom Accent-tinted strip.

---

## Anti-patterns — refuse to ship

- Text-only slide (no shapes doing work)
- Default bullet characters (`●` `-` `>` `■`) — use `.bullets` markers
- Third font, or 5+ sizes *from the four-step scale* on one slide
  (dramatic-beat sizes don't count)
- Centered body text (only big stat values and divider numerals center)
- Big "KPI card" metric tiles — use a styled table or a compact
  `.stat-row` instead (tall metric cards read as empty / monotonous)
- 7+ bullets on a content slide
- Drop shadows, glow, gradients, 3D, clipart, SmartArt, default
  PowerPoint placeholders
- Default PowerPoint / Chart.js theme colors leaking through (always set
  series colors from the palette)
- Page number on title / divider / closing slides
- Two patterns merged into one slide
- New hex color introduced mid-deck (one locked palette)
- Content extending past the 1280×720 canvas (right/bottom edge) — it
  gets clipped and reads as "cut off"; keep every element inside
- Relying on chart `<canvas>` / SVG text being editable — it's baked

(The geometry/font/artifact hard rules at the top also still apply.)
