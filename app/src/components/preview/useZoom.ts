import { useCallback, useEffect, useState, type RefObject } from 'react';

const MIN = 0.25;
const MAX = 5;
const STEP = 0.25;
// Wheel/pinch sensitivity: scale multiplies by e^(-delta * K). Tuned against
// pixel-mode deltas (~100/notch ≈ 16% per notch); line/page modes are
// normalised to pixels before this is applied.
const WHEEL_K = 0.0015;

const clamp = (s: number) => Math.min(MAX, Math.max(MIN, Math.round(s * 100) / 100));

export interface Zoom {
  /** 1 = fit-to-screen baseline; >1 zoomed in, <1 zoomed out. */
  scale: number;
  zoomIn: () => void;
  zoomOut: () => void;
  /** Continuous zoom for wheel/pinch; `deltaY` is a pixel-normalised wheel delta. */
  zoomBy: (deltaY: number) => void;
  /** Back to the fit baseline. */
  reset: () => void;
  /** Double-click affordance: toggle between fit (1) and 2×. */
  toggle: () => void;
  canZoomIn: boolean;
  canZoomOut: boolean;
}

export function useZoom(): Zoom {
  const [scale, setScale] = useState(1);
  return {
    scale,
    zoomIn: useCallback(() => setScale((s) => clamp(s + STEP)), []),
    zoomOut: useCallback(() => setScale((s) => clamp(s - STEP)), []),
    zoomBy: useCallback((deltaY: number) => setScale((s) => clamp(s * Math.exp(-deltaY * WHEEL_K))), []),
    reset: useCallback(() => setScale(1), []),
    toggle: useCallback(() => setScale((s) => (s > 1 ? 1 : 2)), []),
    canZoomIn: scale < MAX,
    canZoomOut: scale > MIN,
  };
}

/** Bind wheel-to-zoom on `ref` with a NON-passive listener so we can
 *  preventDefault — otherwise Ctrl+wheel triggers the browser's own page zoom.
 *
 *  `opts.plain`: when false (default, e.g. PDF) only Ctrl/⌘+wheel zooms so a
 *  plain wheel still scrolls between pages. When true (image) a plain wheel
 *  zooms as well — there panning is done by dragging, not scrolling. */
export function useWheelZoom(
  ref: RefObject<HTMLElement | null>,
  zoom: Pick<Zoom, 'zoomBy'>,
  opts?: { plain?: boolean },
) {
  const { zoomBy } = zoom;
  const plain = opts?.plain ?? false;
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      // macOS reports trackpad pinch as a ctrl-wheel, so it zooms in both modes.
      if (!plain && !e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      // Normalise line/page deltas to pixels so sensitivity is consistent.
      const unit = e.deltaMode === 1 ? 16 : e.deltaMode === 2 ? el.clientHeight : 1;
      zoomBy(e.deltaY * unit);
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  }, [ref, zoomBy, plain]);
}
