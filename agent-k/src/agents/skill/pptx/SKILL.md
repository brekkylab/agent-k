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

---

## Process — outline first, then transcribe to pptx

Two phases. **Separate content from layout.** Write everything that
will appear on the slides as markdown first; then build the deck by
transcribing that markdown into python-pptx.

### 1. Outline (`outline.md`) — verbatim slide content

The outline contains the *actual content* that will appear on each
slide. It's not a plan or summary — it's the source of truth. One
section per slide:

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
  matplotlib PNG and inserted.

A 10-slide outline runs ~200–400 lines. Save as `outline.md` in the
working directory before any python-pptx work.

### 2. Transcribe outline → pptx

Read each `outline.md` section and emit one slide. **Transcribe text
verbatim — don't rewrite, paraphrase, or shorten.** Pick the layout
grammar from the section's content shape (KPI showcase for KPI
tables, card grid for parallel items, asymmetric split for chart +
sidebar) and vary across the deck. Render chart PNGs in matplotlib
with the deck's palette, then `add_picture()`. Apply the locked
palette, type scale, margins, and footer to every slide — consistency
lives here, not in the content.

### 3. Verify the output renders correctly

**Two layers** — a fast automated check and a visual confirmation.

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
its flagged issues, render to PDF + PNG and look:

```
soffice --headless --convert-to pdf out.pptx
pdftoppm -r 100 -png out.pdf slide
```

Then read the PNGs and inspect the rendered content — confirming the
files exist isn't the check; looking at each image is. Catches CJK
tofu (`□□□`), accidental PowerPoint-default chart colors, and the
overlap cases that the verify heuristic only *suspects*.

If the sandbox lacks tooling: `apt-get install -y libreoffice-impress
libreoffice-core poppler-utils`. If anything is off, fix and re-run
from step 2.

---

## Palette — color roles in three tiers

The *role structure* is fixed. The *hex values* are chosen per deck
(matched to subject and mood) and frozen for that deck's lifetime.

| Tier | Role | Job |
|---|---|---|
| Colored fills | **Primary** | Anchor color. Slide titles, section header backgrounds, deepest text. |
| | **Secondary** | Principal accent. Left edge strip, KPI top strips, first chart series, category tags. |
| | **Accent** | Warm pop. Eyebrow tags, corner blocks, quote strip, "current" timeline marker. Use sparingly. |
| Surface tiers | **Background** | Page background. Near-white (light) or near-black / navy (dark). |
| | **Surface** | Card / tile fill. Subtly lifts off Background. |
| | **Tinted Panel** | Surface mixed ~5–10% toward Accent. For callout / insight bands — a different layer from regular Surface. |
| Structural | **Muted** | Supporting text — subtitles, captions, footers, axis labels, attribution. |
| | **Hairline** | Structural lines only — dividers, footer lines, card borders, timeline tracks. |

**Status colors** (deltas only, never a fill):

- **Positive** — green, for upward changes (`+18%`)
- **Negative** — red, for downward changes (`-3d`)

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
- **CJK / mixed-script decks** — default to `Noto Sans CJK KR` / `JP`
  / `SC` (one font across container and pptx — see Rendering notes
  for setup). `Pretendard` only for brand-critical Korean decks where
  the opener has it installed.

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
  right (caption muted). Optional short deck title left
- **Content area** — 12.13" × 5.0"

Title slide, section dividers, quote slide, and closing slide
intentionally break the grid — those are the dramatic beats.

**Asymmetric margins by intent.** Equal margins on all four sides
read "safely centered" — fine for academic decks, monotonous
elsewhere. Pick *one* axis of asymmetry per deck (wider-left + tighter-
right, or wider-bottom + tighter-top, etc.) and apply it consistently.
Don't randomize per slide.

---

## Shape vocabulary — six primitive shapes

Higher-level constructs (KPI cards, insight bands, action pills) are
compositions of these primitives — not new vocabulary. If a slide
needs a shape outside this list, you are inventing.

| # | Shape | Use |
|---|---|---|
| 1 | **Anchor block** — solid rectangle ~4.5" × 2.5", Secondary | Title & closing slides only — deck signature |
| 2 | **Left edge strip** — 0.15" wide, full height, Secondary | Content slides only — pattern identifier |
| 3 | **Hairline** — 0.04" line, Hairline color | Footer, column divider, timeline track, card border. Structural only. |
| 4 | **Bullet marker** — 0.18" filled square in Accent (circle in Primary on Mono) | Replaces default `●` everywhere |
| 5 | **Category tag** — rounded rect 1.4" × 0.4", Secondary or Accent fill, white caption text | Two-column headers |
| 6 | **Event marker** — 0.35" filled circle (Secondary, or Accent for "current") with 0.55" Surface ring | Timeline markers |

Banned effects and other refusals live in **Anti-patterns** below.

---

## Rendering notes — correctness, not style

These aren't design choices; they're things python-pptx leaves to you
that quietly ruin a deck if missed.

### CJK text — install Noto CJK, then use it everywhere

`font.name` only sets the Latin typeface; the East-Asian face
(`<a:ea>` in the run XML) stays unset and PowerPoint falls back to
whatever the opener's machine defaults to. For every CJK-containing
run, set both Latin and East-Asian typefaces in a helper.

**One font, everywhere** — eliminates mismatch between the matplotlib
chart renderer and the pptx text runs:

1. `apt-get install -y fonts-noto-cjk` — covers Korean / Japanese /
   Simplified Chinese in one package.
2. For pptx EA typeface, use `'Noto Sans CJK KR'` (or `JP` / `SC`).
3. For matplotlib `rcParams['font.family']`, use the same name.

Without it, matplotlib chart labels render as tofu (`□□□`) and the
verify-step PDF inherits the same.

Most modern openers (Windows 10+, Mac, Linux) ship with the Noto CJK
family or fall back cleanly. For brand-critical decks where the opener
has Pretendard installed, install Pretendard in the container too
(`wget` from `orioncactus/pretendard` GitHub releases) and swap both
references to `'Pretendard'`.

### Charts — pre-render in matplotlib, insert as PNG
python-pptx's native chart engine has weak CJK axis-font control,
leaks default chart colors despite recoloring, and renders differently
across PowerPoint versions. Render in matplotlib using the deck's
palette, save as PNG, and `add_picture()` it onto the slide.

### Text-box sizing — measure before placing
python-pptx doesn't render text; an undersized box silently clips
when PowerPoint opens. Korean / CJK glyphs visually occupy ~2× ASCII
width, so a 2.5" box holds ~14 Korean chars per line at 14 pt body,
*not* 30. Before placing bullet / card / table-cell text, estimate
the required height — use `box_height_in()` and `line_count()` from
`verify_pptx.py` (same CJK-aware formula the Step-3 overflow check
runs). Skip for hero text (title / Number Cover / quote glyph) where
text drives the box rather than the reverse.

### Other pitfalls
Every shape inherits a default shadow on creation — set
`shape.shadow.inherit = False` on every shape or the banned effects
sneak back. Layer order: place full-slide background, anchor blocks,
edge strips, and card fills *before* any text or the text disappears
under shapes drawn after it.

---

## Layout grammar — vary the division of the content area

A deck where every slide divides its content area the same way reads
monotonously. **Vary the layout slide-to-slide.** The categories
below are *principles for thinking about division*, not templates to
trace. Within each category, compose freshly each time — the sizes,
gutters, and emphasis points should differ per slide.

- **Card grid** — 2–4 equal-size cards in a row (or 2×2). Best for
  parallel concepts: KPIs side-by-side, region snapshots, options
  comparison. Vary *how many* cards (2 vs 3 vs 4) across the deck so
  consecutive grid slides don't feel identical.
- **Asymmetric split** — one side takes ~60–70%, the other ~30–40%.
  Best for: a visualization + supporting facts; a main story + a
  sidebar of callouts. Avoid 50/50 unless the two halves are
  genuinely equal in weight. Flip the heavy side left-vs-right across
  the deck so consecutive split slides don't all face the same way.
- **Stacked rows** — one full-width concept above, another below
  (e.g. top: chart, bottom: takeaway cards). Best for: a fact + its
  implication; two unrelated facts on one topic.
- **Hero + small** — one oversized element (chart, big number, quote
  glyph) + small supporting elements around it. Best for: the deck's
  anchor slides — pivot points the audience should pause on.
- **Two-column comparison** — equal columns separated by a hairline.
  Best for: build vs buy, before vs after, wins vs concerns. Keep the
  left/right meaning consistent across all comparison slides in the
  deck.
- **Timeline** — horizontal track with evenly-spaced markers. Best
  for: sequences, roadmaps, milestone progressions.

**Insight band — optional.** A Tinted Panel strip at the bottom of a
content slide, carrying a one-line takeaway (caption-Accent label +
body-Primary sentence). Use when the slide *makes a claim* ("growth
slowed in Q2"); skip when the slide is just exposition or when the
slide's title already states the claim. Don't put one on every slide
— it stops landing.

**Variation budget.** If three consecutive slides use the same
division (e.g. card grid → card grid → card grid), switch the next
one to a different category. Repetition is fine when intentional
(four region cards across four region slides), but accidental
repetition reads as monotony. A 10-slide content section should
touch at least three different division categories.

---

## Slide patterns

Nine base patterns (each with one or two variants). Pick by the
slide's *job*, not by what looks pretty.

### Title slide — opens the deck

The title slide is the deck's visual signature. **Compose freshly
each time** — there is no canonical layout. Two decks from this
skill, viewed side-by-side, should read as "from the same system"
*only* in palette / typography / footer — never in title-slide
layout. If the safest-feeling choice is Corner Anchor, deliberately
reach for something else; that default is the AI tell.

**Required principles** (these must hold; everything else is open):

1. **One visual anchor** — a single dominant element that establishes
   the deck's identity. Pick a type from the sketch library below.
   Required — without it the slide drifts.
2. **Three-tier text hierarchy** — eyebrow (caption, Accent) → title
   (Display, Primary) → subtitle (Body, Muted). Order is fixed;
   spatial arrangement is open. Eyebrow or subtitle may be omitted on
   Quote / Number / Photo Cover variants where the anchor *is* the
   headline.
3. **One signature mark** — a small Accent shape balancing the visual
   anchor (often in the opposite corner). The closing slide mirrors
   this mark.
4. **No page number.**

#### Sketch library — starting ideas, not templates

Use these as *seeds*. Mix elements freely; invent variations.

- **Corner Anchor** — filled rectangle in one corner + diagonal Accent square + left-aligned text stack.
- **Dark Cover** — Primary BG + off-canvas soft circles + light text. Works on any palette by recoloring.
- **Side Panel** — vertical ~1/3 Primary panel, text inside; remaining ~2/3 mostly empty.
- **Centered Hero** — no shape anchor; the title's typography *is* the anchor. Wide Accent strip beneath.
- **Full-Bleed Type** — title at 80–96 pt fills 70–80% of slide width; eyebrow / subtitle tucked into a corner.
- **Number Cover** — single huge stat / numeral (180–250 pt) as the anchor; title becomes a smaller caption. Best for executive summaries.
- **Photo Cover** — full-bleed image or Primary-toned gradient; overlay text on a thin Tinted Panel for legibility.
- **Quote Cover** — open with a pulled quote (Title size) + attribution; defer the "this is X deck" framing to slide 2.
- **Diagram Cover** — single iconic mark / logo / shape dominates; title is a small annotation. Best when the deck has a natural visual hook.

### Section divider — chapter break
Full-bleed Primary background. Huge numeral ("01") at left — *much
larger than Display*, roughly 3–4× the title size, in a lightened
Secondary tone (scenography, not text the reader processes). Section
title at right, Display, white. ≈0.8" × 0.08" Accent strip beneath.
Optional kicker (caption, Accent) above title. No page number.

### Content slide — a list of points
Left edge strip (shape 2). Optional kicker (caption, Secondary) just
above the title; Title at the top of the title band (Title size,
Primary). **4–6 bullets max**, each with bullet marker (shape 4) at
left, generous line gap (~0.35"). Seven bullets means two are
duplicates — merge or split.

### Two-column comparison
Left edge strip + title band as content. Vertical hairline (shape 3)
splits content area. Category tag (shape 5) atop each column —
Secondary fill left, Accent fill right. 3–5 bullets per column with
markers. Keep left/right color mapping consistent across all
comparison slides in the deck.

**Wins / concerns variant.** For qualitative comparisons (worked vs
needs attention), drop the category tag, use a caption-sized section
heading per column (Secondary left, Negative/Accent right), and swap
bullet markers for ✓ / ✕ glyphs.

### KPI showcase — headline numbers
Title band. **2–4 cards** across content area, 0.3" gutters. Each
card: Surface fill, no shadow, 0.5pt Hairline border, 0.12" Secondary
top strip. Inside: Display value (44 pt if 4 cards), Primary;
caption-uppercase label, Muted, below; optional delta (caption) —
Positive for leading `+`, Negative for `-`.

**Card-strip orientation** — pick one per deck. *Top strip*
(default; Bloomberg/newsroom feel) = square corners + 0.12" Secondary
strip top edge. *Left strip* (startup/IR feel) = rounded corners
(radius ≈ 0.09") + 0.10" Secondary strip left edge + Tinted Panel
fill. Mixing reads as inconsistency.

### Quote slide — a single thought
Background fill. Oversized `"` glyph top-left (≈4× Display size),
Accent mixed toward Background (decoration, not character). Quote at
Title size, Primary, line-height 1.2, ≤ 4 lines (more → it's an
essay, cut it). ≈0.5" × 0.06" Accent strip below. Attribution (Body,
Muted) right-aligned to strip's right edge.

### Chart slide — data
Title band. **Title = takeaway** ("Paid users overtook free in Q2"),
not subject ("MAU by tier"). Optional Body-Muted narrative below.
Chart fills content area. **Series palette = Secondary, Accent,
Primary in that order; max three series** (fourth → should be a
table). Axis text caption-Muted. Gridlines off unless precision
matters — prefer data labels.

### Timeline — events in sequence
Title band. Horizontal hairline across vertically-centered content
area = the track. **3–6 event markers** (shape 6) evenly spaced.
Above: period label (caption, Accent, e.g. "Q1"). Below: milestone
label (Body bold, Primary) + optional caption-Muted detail. One
marker may swap to Accent for "current."

### Closing slide — ends the deck

Mirrors whichever title composition the deck opened with — same
palette, same shapes, position flipped (opposite corner / opposite
quadrant / opposite panel side / accent strip above instead of
below). Display message ("Thank you.", "Questions?", "Let's build
it.") in the title's position-equivalent. No page number.

**Action-plan variant.** When the deck demands explicit next steps,
swap the soft closing for a dark Primary background: title band at
top (Display, white); left column of 4–6 numbered action cards
(horizontal pills, Accent-numbered circle, darker-Primary fill);
right column of 4–6 risk bullets (light-gray); bottom Accent-tinted
"Decisions needed" strip with 3–5 inline items separated by `·`.

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

---

A quiet, consistent deck reads as designed. A loud, varied deck reads
as assembled.
