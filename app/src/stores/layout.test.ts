import { describe, expect, it } from 'vitest';
import {
  getSidebarModeForDrag,
  getSidebarModeWhileResizing,
  isSidebarRevealHoldPoint,
  shouldCloseSidebarRevealOnNavigation,
  shouldRevealSidebarAfterDrag,
  SIDEBAR_EXPAND_ABOVE,
  SIDEBAR_HIDDEN_BELOW,
  SIDEBAR_MOBILE_BREAKPOINT,
  SIDEBAR_REVEAL_EXIT_BUFFER,
  SIDEBAR_REVEAL_WIDTH,
} from './layout';

describe('sidebar layout thresholds', () => {
  it('uses separate thresholds for hiding and expanding', () => {
    expect(getSidebarModeForDrag(SIDEBAR_HIDDEN_BELOW - 1, false)).toBe('hidden');
    expect(getSidebarModeForDrag(SIDEBAR_HIDDEN_BELOW, false)).toBe('expanded');
    expect(getSidebarModeForDrag(SIDEBAR_EXPAND_ABOVE - 1, true)).toBe('hidden');
    expect(getSidebarModeForDrag(SIDEBAR_EXPAND_ABOVE, true)).toBe('expanded');
  });

  it('keeps the sidebar expanded while the resizer is actively held', () => {
    expect(getSidebarModeWhileResizing('hidden')).toBe('expanded');
    expect(getSidebarModeWhileResizing('expanded')).toBe('expanded');
  });

  it('keeps reveal open through the configured right-side buffer', () => {
    const holdX = SIDEBAR_REVEAL_WIDTH + SIDEBAR_REVEAL_EXIT_BUFFER;

    expect(holdX).toBe(230);
    expect(isSidebarRevealHoldPoint(holdX, 100, 720)).toBe(true);
    expect(isSidebarRevealHoldPoint(holdX + 1, 100, 720)).toBe(false);
  });

  it('keeps desktop hidden reveal open across navigation', () => {
    expect(shouldCloseSidebarRevealOnNavigation('hidden', SIDEBAR_MOBILE_BREAKPOINT)).toBe(false);
    expect(shouldCloseSidebarRevealOnNavigation('hidden', SIDEBAR_MOBILE_BREAKPOINT - 1)).toBe(true);
    expect(shouldCloseSidebarRevealOnNavigation('expanded', SIDEBAR_MOBILE_BREAKPOINT)).toBe(true);
  });

  it('keeps reveal visible when dragging into hidden while the cursor is inside it', () => {
    expect(shouldRevealSidebarAfterDrag('hidden', SIDEBAR_REVEAL_WIDTH - 1, 100, 720)).toBe(true);
    expect(shouldRevealSidebarAfterDrag('hidden', SIDEBAR_REVEAL_WIDTH + SIDEBAR_REVEAL_EXIT_BUFFER + 1, 100, 720)).toBe(false);
    expect(shouldRevealSidebarAfterDrag('expanded', SIDEBAR_REVEAL_WIDTH - 1, 100, 720)).toBe(false);
  });
});
