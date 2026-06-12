import { useEffect, useRef, useState } from 'react';

export interface MarqueeRect {
  left: number; top: number; width: number; height: number;
}

interface UseMarqueeSelectionOpts {
  /** Scroll container. The anchor is pinned to its content, so scrolling mid-drag
   *  keeps selecting the rows that scroll past. */
  scrollRef: React.RefObject<HTMLElement | null>;
  /** CSS selector for one selectable row — used both to detect a press that lands
   *  on a row and to enumerate rows for hit-testing. */
  itemSelector: string;
  /** dataset key holding a row's selection key (e.g. 'sfPath' for data-sf-path). */
  keyAttr: string;
  /** Current selection, read once at press to seed an additive / row-start drag. */
  getSelection: () => Set<string>;
  /** Replace the selection (called as the marquee sweeps). */
  setSelection: (next: Set<string>) => void;
  /** Pressing empty space with no modifier; defaults to clearing the selection.
   *  Provide this when clearing must also reset other state (e.g. a shift anchor). */
  onClear?: () => void;
  /** Elements that must NOT start a marquee (e.g. an inline action button). */
  ignoreSelector?: string;
  /** Pixels of movement before a press becomes a drag. */
  threshold?: number;
}

/**
 * Rubber-band (marquee) multi-selection for a scrollable list of rows. The caller
 * owns the selection state; this hook wires the press/drag/scroll/release handlers
 * and reports the rectangle to draw. Returns `cancel` so a native HTML5 drag that
 * starts on a row can abort a pending marquee.
 */
export function useMarqueeSelection(opts: UseMarqueeSelectionOpts) {
  const optsRef = useRef(opts);
  optsRef.current = opts;

  const [dragRect, setDragRect] = useState<MarqueeRect | null>(null);
  const originRef = useRef<{ x: number; y: number; base: Set<string> } | null>(null);
  const didDragRef = useRef(false);
  const lastPointerRef = useRef({ x: 0, y: 0 });
  const lastScrollTopRef = useRef(0);
  // The capture-phase click listener onUp installs to swallow the post-marquee
  // click. Tracked here so the effect cleanup can remove it if the component
  // unmounts before that click arrives (otherwise it leaks on window).
  const swallowRef = useRef<((ev: Event) => void) | null>(null);

  function onMouseDown(e: React.MouseEvent) {
    if (e.button !== 0) return;
    const o = optsRef.current;
    const target = e.target as HTMLElement;
    if (o.ignoreSelector && target.closest(o.ignoreSelector)) return;
    const onItem = !!target.closest(o.itemSelector);
    const additive = e.shiftKey || e.metaKey || e.ctrlKey;
    originRef.current = {
      x: e.clientX,
      y: e.clientY,
      base: onItem || additive ? new Set(o.getSelection()) : new Set(),
    };
    lastPointerRef.current = { x: e.clientX, y: e.clientY };
    lastScrollTopRef.current = o.scrollRef.current?.scrollTop ?? 0;
    didDragRef.current = false;
    if (!onItem && !additive) {
      if (o.onClear) o.onClear();
      else o.setSelection(new Set());
    }
  }

  function cancel() {
    originRef.current = null;
    setDragRect(null);
  }

  useEffect(() => {
    function apply(cx: number, cy: number) {
      const origin = originRef.current;
      if (!origin) return;
      const o = optsRef.current;
      const threshold = o.threshold ?? 4;
      const dx = cx - origin.x;
      const dy = cy - origin.y;
      if (Math.abs(dx) < threshold && Math.abs(dy) < threshold && !dragRect) return;
      didDragRef.current = true;
      const left = Math.min(origin.x, cx);
      const top = Math.min(origin.y, cy);
      const width = Math.abs(dx);
      const height = Math.abs(dy);
      const r = { left, top, right: left + width, bottom: top + height };
      // Draw the full rect (portaled to <body>); it can roam the whole screen,
      // matching the original Files-page behavior. The hit-test below uses the
      // same rect, so rows scrolled out of the list still select.
      setDragRect({ left, top, width, height });
      const next = new Set(origin.base);
      for (const el of document.querySelectorAll<HTMLElement>(o.itemSelector)) {
        const rect = el.getBoundingClientRect();
        if (rect.left < r.right && rect.right > r.left && rect.top < r.bottom && rect.bottom > r.top) {
          const key = el.dataset[o.keyAttr];
          if (key) next.add(key);
        }
      }
      o.setSelection(next);
    }
    function onMove(e: MouseEvent) {
      lastPointerRef.current = { x: e.clientX, y: e.clientY };
      apply(e.clientX, e.clientY);
    }
    // Scrolling mid-drag: the anchor is pinned to content, so shift it by the
    // scroll delta and re-test (a wheel scroll fires no mousemove).
    function onScroll() {
      const origin = originRef.current;
      const sc = optsRef.current.scrollRef.current;
      if (!origin || !sc) return;
      const delta = sc.scrollTop - lastScrollTopRef.current;
      lastScrollTopRef.current = sc.scrollTop;
      origin.y -= delta;
      apply(lastPointerRef.current.x, lastPointerRef.current.y);
    }
    function onUp() {
      const dragged = didDragRef.current && originRef.current;
      originRef.current = null;
      setDragRect(null);
      if (dragged) {
        // Swallow the click that follows a marquee so it doesn't reset selection.
        const swallow = (ev: Event) => {
          ev.stopPropagation();
          ev.preventDefault();
          window.removeEventListener('click', swallow, true);
          swallowRef.current = null;
        };
        swallowRef.current = swallow;
        window.addEventListener('click', swallow, true);
      }
    }
    const sc = optsRef.current.scrollRef.current;
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
    sc?.addEventListener('scroll', onScroll);
    return () => {
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
      sc?.removeEventListener('scroll', onScroll);
    };
  }, [dragRect]);

  // Remove a still-pending post-marquee swallow listener on unmount only. The
  // effect above re-runs on every dragRect change — including the setDragRect(null)
  // in onUp — so its cleanup must NOT touch the swallow, or the listener gets torn
  // down before the click it exists to swallow, and that click clears the selection.
  useEffect(() => () => {
    if (swallowRef.current) {
      window.removeEventListener('click', swallowRef.current, true);
      swallowRef.current = null;
    }
  }, []);

  return { onMouseDown, dragRect, cancel };
}
