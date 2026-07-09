# Design

Visual system for app_v2 — the single-user instance of the agent-k cowork
platform. app_v2 is a **sibling** of the team app: identical structural design
system (typography, spacing, radii, layout, component vocabulary, motion) with a
**deliberately distinct color identity**. All color decisions live in one token
layer, `src/styles/theme.css`, loaded after `globals.css`.

## Theme

Light, cool, single-user workspace.

**Scene sentence:** one knowledge worker at a desk, indoors under cool daytime /
screen light, calm and self-directed, driving their own agent sessions. That
scene forces a quiet, cool surface (not the team app's warm cream) with a single
confident accent for the things they act on — send, open, select, focus.

**Divergence from the team app** (the identity we are a sibling of):

| Axis     | Team app (sibling)                | app_v2 (this instance)                  |
|----------|-----------------------------------|-----------------------------------------|
| Neutrals | warm cream, paper hue ~75, ink ~60| cool slate, paper + ink hue ~265–275    |
| Accent   | deep slate-teal, hue ~215         | committed indigo-violet, hue ~278       |
| Feel     | warm, communal, team              | cool, solitary, personal                |

Same paper→ink ramp structure, same accent-soft/accent-ink badge pattern, same
semantic vocabulary — only the hues and temperature move. This keeps app_v2
unmistakably part of the family while reading as its own instance at a glance.

**Color strategy:** Restrained (product floor). Tinted-cool neutrals carry the
surface; one indigo-violet accent (< 10% of surface) marks primary actions,
focus, active selection, and state. No secondary decorative color.

## Color Palette

OKLCH throughout. Tokens defined in `src/styles/theme.css`.

### Surfaces — cool slate paper ramp (hue 265, very low chroma)

| Token            | OKLCH                     | Role                              |
|------------------|---------------------------|-----------------------------------|
| `--cw-paper`     | `oklch(0.982 0.004 265)`  | main page background              |
| `--cw-paper-2`   | `oklch(0.968 0.006 265)`  | card / pane / form panel bg       |
| `--cw-paper-3`   | `oklch(0.945 0.008 265)`  | hover, inset, chip, code bg       |
| `--cw-paper-4`   | `oklch(0.918 0.010 265)`  | deeper inset, divider hover       |
| `--cw-neutral-0` | `oklch(1 0 0)`            | pure white surface (dialogs)      |

### Borders

| Token              | OKLCH                    | Role                        |
|--------------------|--------------------------|-----------------------------|
| `--cw-line`        | `oklch(0.898 0.010 265)` | default border              |
| `--cw-line-strong` | `oklch(0.855 0.012 265)` | input / control border      |
| `--cw-line-soft`   | `oklch(0.940 0.008 265)` | soft divider (list rows)    |

### Ink — cool slate text ramp (hue ~270)

| Token         | OKLCH                    | Role                            |
|---------------|--------------------------|---------------------------------|
| `--cw-ink`    | `oklch(0.205 0.012 275)` | primary text                    |
| `--cw-ink-2`  | `oklch(0.375 0.014 272)` | secondary text, nav link        |
| `--cw-ink-3`  | `oklch(0.545 0.013 270)` | tertiary / muted (dates, hints) |
| `--cw-ink-4`  | `oklch(0.700 0.010 268)` | disabled / decorative only      |

### Accent — committed indigo-violet (hue 278)

| Token             | OKLCH                    | Role                             |
|-------------------|--------------------------|----------------------------------|
| `--cw-accent`     | `oklch(0.500 0.170 278)` | primary action, focus, active    |
| `--cw-accent-2`   | `oklch(0.430 0.175 278)` | hover / pressed accent           |
| `--cw-accent-soft`| `oklch(0.955 0.020 278)` | accent pill / badge bg           |
| `--cw-accent-ink` | `oklch(0.320 0.070 278)` | text on accent-soft (badge)      |
| `--cw-on-accent`  | `oklch(1 0 0)`          | text on solid accent (white)     |

### Semantic

| Token                   | OKLCH                    | Role                        |
|-------------------------|--------------------------|-----------------------------|
| `--cw-destructive`      | `oklch(0.560 0.190 27)`  | error text, delete action   |
| `--cw-destructive-2`    | `oklch(0.480 0.185 27)`  | destructive hover           |
| `--cw-destructive-soft` | `oklch(0.968 0.020 27)`  | error box bg                |
| `--cw-ok`               | `oklch(0.560 0.130 150)` | connected status dot        |
| `--cw-warn`             | `oklch(0.720 0.150 70)`  | disconnected status dot     |
| `--cw-warn-bg`          | `oklch(0.972 0.028 85)`  | warning box bg              |
| `--cw-warn-fg`          | `oklch(0.470 0.080 65)`  | warning box text            |
| `--cw-scrim`            | `oklch(0 0 0 / 0.42)`    | dialog backdrop             |

## Contrast (WCAG 2.1 AA)

Computed OKLCH → sRGB → relative luminance. Body >= 4.5:1, large/UI >= 3:1.

| Pair                                  | Ratio    | Result           |
|---------------------------------------|----------|------------------|
| ink on paper (body)                   | 17.0:1   | PASS AA          |
| ink-2 on paper (secondary)            | 9.7:1    | PASS AA          |
| ink-3 on paper (tertiary / muted)     | 4.7:1    | PASS AA          |
| ink-3 on paper-2                      | 4.5:1    | PASS AA          |
| white on accent (primary button)     | 6.3:1    | PASS AA          |
| white on accent-2 (button hover)      | 8.7:1    | PASS AA          |
| accent-ink on accent-soft (badge)     | 11.3:1   | PASS AA          |
| accent on paper (link / active)       | 6.0:1    | PASS AA          |
| white on destructive                  | 5.1:1    | PASS AA          |
| destructive on destructive-soft (box) | 4.6:1    | PASS AA          |
| warn-fg on warn-bg (warning box)      | 6.4:1    | PASS AA          |

`--cw-ink-4` (2.5:1 on paper) is reserved for disabled / decorative use only,
which WCAG exempts; it is never used for essential body text.

## Typography, Spacing, Layout, Motion

Unchanged from base / the team app. `system-ui, -apple-system, sans-serif`,
fixed rem scale, existing spacing rhythm and radii. This task changed color
tokens only.

## Token Architecture

- All color values live in `src/styles/theme.css` (`:root` custom properties).
- `globals.css` references tokens via `var(--cw-*, <literal fallback>)`; the
  fallback is the original ported literal so the base file stays self-consistent
  if the theme layer is ever removed.
- Fixed dark-overlay surfaces in the file-preview lightbox (white text on a
  black scrim, white document sheets) intentionally keep literal colors — they
  are content surfaces, not themable app chrome, matching the sibling app.
- To restyle: edit `theme.css` only. Do not add raw colors to `globals.css`.
