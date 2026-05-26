import { describe, expect, it } from 'vitest';
import {
  getSidebarModeForDrag,
  isSidebarRevealHoldPoint,
  SIDEBAR_HIDDEN_BELOW,
  SIDEBAR_REVEAL_EXIT_BUFFER,
  SIDEBAR_REVEAL_WIDTH,
} from './layout';

describe('sidebar layout thresholds', () => {
  // Option B from the Notion/Outline review: shrink-to-hide stays one-way,
  // expand-from-hidden does NOT auto-pin (handled by an explicit toggle button).
  it('snaps to hidden only when drag drops below the shrink threshold', () => {
    expect(getSidebarModeForDrag(SIDEBAR_HIDDEN_BELOW - 1)).toBe('hidden');
    expect(getSidebarModeForDrag(SIDEBAR_HIDDEN_BELOW)).toBe('expanded');
    // Wide drags stay expanded — there is no hysteresis flipping mode back and forth.
    expect(getSidebarModeForDrag(300)).toBe('expanded');
  });

  it('keeps reveal open through the configured right-side buffer', () => {
    // floatingWidth is the live panel width (= expandedWidth). The hold region
    // is the panel width plus the configured exit buffer, inclusive on the
    // right edge, exclusive after. clientY is not part of the contract.
    const floatingWidth = SIDEBAR_REVEAL_WIDTH;
    const holdX = floatingWidth + SIDEBAR_REVEAL_EXIT_BUFFER;

    expect(isSidebarRevealHoldPoint(holdX, floatingWidth)).toBe(true);
    expect(isSidebarRevealHoldPoint(holdX + 1, floatingWidth)).toBe(false);
    expect(isSidebarRevealHoldPoint(-1, floatingWidth)).toBe(false);

    // Larger panel → larger hold region, automatically.
    const wider = 300;
    expect(isSidebarRevealHoldPoint(wider + SIDEBAR_REVEAL_EXIT_BUFFER, wider)).toBe(true);
    expect(isSidebarRevealHoldPoint(wider + SIDEBAR_REVEAL_EXIT_BUFFER + 1, wider)).toBe(false);
  });
});
