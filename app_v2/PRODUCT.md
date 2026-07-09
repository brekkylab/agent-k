# Product

## Register

product

## Users

A single knowledge worker running their own agent sessions from a desk, indoors,
under cool daytime or screen light. They are always in a task: starting a
session, chatting with the agent, watching a run stream, browsing the files the
agent produced. There is no team, no invites, no shared project — this is the
single-user instance of the cowork platform. The user trusts the tool to stay
out of the way while they think.

## Product Purpose

app_v2 is the single-user surface of the agent-k cowork platform: sessions +
chat with SSE streaming, plus a workspace file browser with rich previews. It
exists so one person can drive coworker agents end-to-end without the team
scaffolding of the sibling app. Success is invisibility — the user gets to the
task (send a message, read the reply, open the artifact) with no friction and no
"is this the right button?" hesitation. A later multi-source Workspace hub will
build on this same shell.

## Brand Personality

Focused, quiet, dependable. Three words: **calm, precise, self-directed.** The
interface should feel like a well-tuned personal workspace, not a marketing
surface and not a busy team dashboard. It is a *sibling* of the team app — same
bones, same craft bar — but its own instance: cooler, more solitary, more
"this is mine."

## Anti-references

- The warm-cream, slate-teal team app — app_v2 must be recognizably NOT that
  instance at a glance (different temperature and accent), while sharing its
  structure. Not a rebrand; a sibling.
- Generic indigo-on-white SaaS boilerplate ("unstyled default with indigo
  buttons") — the starting state we are deliberately moving past.
- Over-decorated productivity tools: gratuitous gradients, glass cards, motion
  for its own sake. The tool disappears into the task.

## Design Principles

- **Sibling, not clone, not stranger.** Keep the team app's token architecture,
  spacing, typography, and component vocabulary; diverge only on color identity
  (temperature + accent hue), and only enough to read as a distinct instance.
- **The tool disappears.** Earned familiarity over novelty. Standard affordances,
  consistent component vocabulary, no invented controls.
- **Color carries state, not decoration.** The accent marks primary actions,
  focus, active selection, and status — nothing purely ornamental.
- **Legibility is non-negotiable.** Body and muted text clear WCAG AA (4.5:1);
  status and semantic colors read at a glance.
- **One place for color.** Every color decision lives in one token layer
  (`src/styles/theme.css`); surfaces reference tokens, never raw literals.

## Accessibility & Inclusion

- WCAG 2.1 AA target. Body text >= 4.5:1, large/UI text >= 3:1 against its
  background; verified per token pair (see DESIGN.md).
- Focus is always visible (2px accent focus-ring, retained from base).
- Color is never the sole state signal where a label or shape can accompany it.
- Motion is minimal and state-driven; no orchestrated load sequences to opt out
  of, but any future motion must honor `prefers-reduced-motion`.
