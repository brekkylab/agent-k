import { createPortal } from 'react-dom';
import type { MarqueeRect } from '@/lib/useMarqueeSelection';

/**
 * Renders the rubber-band rectangle produced by {@link useMarqueeSelection}.
 * Portaled to <body> so a transformed / overflow-hidden ancestor can't re-anchor
 * or clip the fixed overlay, and so the box can extend across the whole screen.
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
      }}
    />,
    document.body,
  );
}
