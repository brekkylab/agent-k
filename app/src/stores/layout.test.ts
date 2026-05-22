import { describe, expect, it } from 'vitest';
import {
  getSidebarModeForDrag,
  isSidebarRevealHoldPoint,
  SIDEBAR_EXPAND_ABOVE,
  SIDEBAR_HIDDEN_BELOW,
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

  it('keeps reveal open through the configured right-side buffer', () => {
    const holdX = SIDEBAR_REVEAL_WIDTH + SIDEBAR_REVEAL_EXIT_BUFFER;

    expect(holdX).toBe(230);
    expect(isSidebarRevealHoldPoint(holdX)).toBe(true);
    expect(isSidebarRevealHoldPoint(holdX + 1)).toBe(false);
    expect(isSidebarRevealHoldPoint(-1)).toBe(false);
  });
});
