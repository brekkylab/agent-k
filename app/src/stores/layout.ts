import { create } from 'zustand';
import { persist } from 'zustand/middleware';

export type SidebarMode = 'expanded' | 'hidden';

export const SIDEBAR_EXPANDED_MIN = 180;
export const SIDEBAR_EXPANDED_MAX = 420;
// Drag must go below this width to snap into hidden mode. The gap between this
// and SIDEBAR_EXPANDED_MIN (180 to 120) acts as a soft wall so the sidebar doesn't
// disappear the moment you reach the minimum width.
export const SIDEBAR_HIDDEN_BELOW = 120;
// Hidden to expanded uses a slightly higher threshold than the expanded minimum
// so the sidebar does not flicker back open while the user is still near the edge.
export const SIDEBAR_EXPAND_ABOVE = 185;
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

export function getSidebarModeForDrag(width: number, hiddenAtStart: boolean): SidebarMode {
  if (hiddenAtStart && width < SIDEBAR_EXPAND_ABOVE) return 'hidden';
  if (width < SIDEBAR_HIDDEN_BELOW) return 'hidden';
  return 'expanded';
}

export function getSidebarModeWhileResizing(modeAfterRelease: SidebarMode): SidebarMode {
  return modeAfterRelease === 'hidden' ? 'expanded' : modeAfterRelease;
}

export function isSidebarRevealHoldPoint(
  clientX: number,
  clientY: number,
  viewportHeight: number,
): boolean {
  return clientX >= 0
    && clientX <= SIDEBAR_REVEAL_WIDTH + SIDEBAR_REVEAL_EXIT_BUFFER
    && clientY >= 0
    && clientY <= viewportHeight;
}

export function shouldCloseSidebarRevealOnNavigation(
  sidebarMode: SidebarMode,
  viewportWidth: number,
): boolean {
  return viewportWidth < SIDEBAR_MOBILE_BREAKPOINT || sidebarMode !== 'hidden';
}

export function shouldRevealSidebarAfterDrag(
  sidebarMode: SidebarMode,
  clientX: number,
  clientY: number,
  viewportHeight: number,
): boolean {
  return sidebarMode === 'hidden' && isSidebarRevealHoldPoint(clientX, clientY, viewportHeight);
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
