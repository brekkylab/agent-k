import { createPortal } from 'react-dom';
import type { MarqueeRect } from '@/lib/useMarqueeSelection';

/**
 * Renders the rubber-band rectangle produced by {@link useMarqueeSelection}.
 * Portaled to <body> so a transformed / overflow-hidden ancestor can't re-anchor
 * or clip the fixed overlay. The border on any clamped edge is dropped so a
 * clamped side doesn't draw a stray line at the scroll container's boundary.
 */
export function MarqueeOverlay({ rect }: { rect: MarqueeRect | null }) {
  if (!rect) return null;
  return createPortal(
    <div
      className="cw-marquee"
      style={{
        left: rect.left,
        top: rect.top,
        width: rect.width,
        height: rect.height,
        borderTopWidth: rect.clampTop ? 0 : undefined,
        borderBottomWidth: rect.clampBottom ? 0 : undefined,
        borderLeftWidth: rect.clampLeft ? 0 : undefined,
        borderRightWidth: rect.clampRight ? 0 : undefined,
      }}
    />,
    document.body,
  );
}
