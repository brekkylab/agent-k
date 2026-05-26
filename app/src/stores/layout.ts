import { create } from 'zustand';
import { persist } from 'zustand/middleware';

export type SidebarMode = 'expanded' | 'hidden';

// Raised from 180 → 200 so the 'Cowork' brand text doesn't visually crunch at
// the minimum resize width. Below this point shrinking only hurts legibility.
export const SIDEBAR_EXPANDED_MIN = 200;
export const SIDEBAR_EXPANDED_MAX = 420;
// Drag must go below this width to snap into hidden mode. The gap between this
// and SIDEBAR_EXPANDED_MIN (200 to 120) acts as a soft wall so the sidebar doesn't
// disappear the moment you reach the minimum width.
export const SIDEBAR_HIDDEN_BELOW = 120;
// Hidden reveal floating width matches the smallest expanded width. The exit
// buffer keeps reveal open just a little past that edge, up to roughly 230px.
export const SIDEBAR_REVEAL_WIDTH = SIDEBAR_EXPANDED_MIN;
export const SIDEBAR_REVEAL_EXIT_BUFFER = 50;
export const SIDEBAR_MOBILE_BREAKPOINT = 768;
const SIDEBAR_EXPANDED_DEFAULT = 240;

interface LayoutState {
  sidebarMode: SidebarMode;
  expandedWidth: number;
  setSidebarMode: (mode: SidebarMode) => void;
  setExpandedWidth: (width: number) => void;
  projectsExpanded: boolean;
  sessionsExpanded: boolean;
  toggleProjects: () => void;
  toggleSessions: () => void;
}

function clampExpanded(width: number): number {
  return Math.min(SIDEBAR_EXPANDED_MAX, Math.max(SIDEBAR_EXPANDED_MIN, width));
}

// Drag only shrinks-into-hidden. Expanding from hidden no longer auto-pins —
// pinning is a deliberate act via the toggle button. So this is one-way:
// expanded → hidden when width drops below the threshold, expanded otherwise.
// Callers should not invoke this from a hidden start state; the resizer is
// hidden in that mode (see Sidebar.tsx / globals.css).
export function getSidebarModeForDrag(width: number): SidebarMode {
  return width < SIDEBAR_HIDDEN_BELOW ? 'hidden' : 'expanded';
}

// Width-only reveal-hold check: clientY is intentionally ignored so the user can
// freely move the cursor above/below the sidebar without losing the floating
// reveal. floatingWidth is the live panel width (expandedWidth) so the hold
// region grows/shrinks with the user-chosen size.
export function isSidebarRevealHoldPoint(clientX: number, floatingWidth: number): boolean {
  return clientX >= 0 && clientX <= floatingWidth + SIDEBAR_REVEAL_EXIT_BUFFER;
}

export const useLayoutStore = create<LayoutState>()(
  persist(
    (set) => ({
      sidebarMode: 'expanded',
      expandedWidth: SIDEBAR_EXPANDED_DEFAULT,
      setSidebarMode: (sidebarMode) => set({ sidebarMode }),
      setExpandedWidth: (width) => set({ expandedWidth: clampExpanded(width) }),
      projectsExpanded: true,
      sessionsExpanded: true,
      toggleProjects: () => set((s) => ({ projectsExpanded: !s.projectsExpanded })),
      toggleSessions: () => set((s) => ({ sessionsExpanded: !s.sessionsExpanded })),
    }),
    { name: 'cowork-layout' },
  ),
);
