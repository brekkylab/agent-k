import { useCallback, useEffect, useState, type RefObject } from 'react';

const MIN = 0.25;
const MAX = 5;
const STEP = 0.25;

const clamp = (s: number) => Math.min(MAX, Math.max(MIN, Math.round(s * 100) / 100));

export interface Zoom {
  /** 1 = fit-to-screen baseline; >1 zoomed in, <1 zoomed out. */
  scale: number;
  zoomIn: () => void;
  zoomOut: () => void;
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
    reset: useCallback(() => setScale(1), []),
    toggle: useCallback(() => setScale((s) => (s > 1 ? 1 : 2)), []),
    canZoomIn: scale < MAX,
    canZoomOut: scale > MIN,
  };
}

/** Bind Ctrl/⌘+wheel (and trackpad pinch, which the browser reports as a
 *  ctrl-wheel) to zoom on `ref`, with a NON-passive listener so we can
 *  preventDefault — otherwise Ctrl+wheel triggers the browser's own page zoom. */
export function useWheelZoom(ref: RefObject<HTMLElement | null>, zoom: Pick<Zoom, 'zoomIn' | 'zoomOut'>) {
  const { zoomIn, zoomOut } = zoom;
  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return; // plain scroll = pan, not zoom
      e.preventDefault();
      if (e.deltaY < 0) zoomIn();
      else if (e.deltaY > 0) zoomOut();
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return () => el.removeEventListener('wheel', onWheel);
  }, [ref, zoomIn, zoomOut]);
}
