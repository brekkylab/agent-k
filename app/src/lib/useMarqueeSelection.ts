import { useEffect, useRef, useState } from 'react';

export interface MarqueeRect {
  left: number; top: number; width: number; height: number;
  /** Sides clamped to the scroll container — render with no border there so a
   *  clamped edge doesn't draw a stray line at the list boundary. */
  clampTop: boolean; clampRight: boolean; clampBottom: boolean; clampLeft: boolean;
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
      // Draw the box clamped to the scroll container so a scroll-extended anchor
      // doesn't paint it over the breadcrumb/header outside the list. The
      // hit-test below still uses the full rect so scrolled-off rows select.
      const sc = o.scrollRef.current;
      if (sc) {
        const b = sc.getBoundingClientRect();
        const vl = Math.max(r.left, b.left);
        const vt = Math.max(r.top, b.top);
        const vr = Math.min(r.right, b.right);
        const vb = Math.min(r.bottom, b.bottom);
        setDragRect({
          left: vl, top: vt, width: Math.max(0, vr - vl), height: Math.max(0, vb - vt),
          clampLeft: r.left < b.left, clampTop: r.top < b.top,
          clampRight: r.right > b.right, clampBottom: r.bottom > b.bottom,
        });
      } else {
        setDragRect({ left, top, width, height, clampLeft: false, clampTop: false, clampRight: false, clampBottom: false });
      }
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
        };
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

  return { onMouseDown, dragRect, cancel };
}
