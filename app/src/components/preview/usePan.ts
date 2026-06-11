import { useEffect, useRef, useState, type RefObject } from 'react';

/**
 * Drag-to-pan for a scrollable stage whose content overflows when zoomed in.
 * A mouse drag that starts on the content translates into scroll offset; pointer
 * capture keeps the gesture alive once the cursor leaves the element.
 *
 * - `pannable` (computed by the caller from its own measurements) gates whether
 *   a drag can start, so this hook owns only the gesture — no second
 *   ResizeObserver or overflow math duplicating the caller's.
 * - Restricted to mouse pointers so touch keeps its native momentum scrolling.
 * - A drag must start on `contentRef` (not the dim margin) — pressing the margin
 *   still falls through to the backdrop's click-to-dismiss. Since the modal
 *   dismisses on `mousedown` over a non-content target, a press that lands on the
 *   content never triggers dismissal, so no click suppression is needed here.
 *
 * Returns `dragging` (true mid-gesture) so the caller can show a grabbing cursor.
 */
export function useDragPan(
  stageRef: RefObject<HTMLElement | null>,
  contentRef: RefObject<HTMLElement | null>,
  pannable: boolean,
) {
  const [dragging, setDragging] = useState(false);
  // Latest `pannable` read inside the long-lived pointerdown listener without
  // re-binding it every render.
  const pannableRef = useRef(pannable);
  pannableRef.current = pannable;

  useEffect(() => {
    const stage = stageRef.current;
    if (!stage) return;
    let startX = 0;
    let startY = 0;
    let startLeft = 0;
    let startTop = 0;
    let active = false;

    const onDown = (e: PointerEvent) => {
      if (e.button !== 0 || e.pointerType !== 'mouse' || !pannableRef.current) return;
      const content = contentRef.current;
      if (!content || !(e.target instanceof Node) || !content.contains(e.target)) return;
      active = true;
      startX = e.clientX;
      startY = e.clientY;
      startLeft = stage.scrollLeft;
      startTop = stage.scrollTop;
      // Keep the gesture alive when the cursor leaves the stage. Guarded because
      // a synthetic pointer (tests) has no active capture target.
      try {
        stage.setPointerCapture(e.pointerId);
      } catch {
        /* no active pointer to capture */
      }
      setDragging(true);
    };
    const onMove = (e: PointerEvent) => {
      if (!active) return;
      stage.scrollLeft = startLeft - (e.clientX - startX);
      stage.scrollTop = startTop - (e.clientY - startY);
    };
    const onUp = (e: PointerEvent) => {
      if (!active) return;
      active = false;
      setDragging(false);
      try {
        stage.releasePointerCapture(e.pointerId);
      } catch {
        /* pointer already released */
      }
    };

    stage.addEventListener('pointerdown', onDown);
    stage.addEventListener('pointermove', onMove);
    stage.addEventListener('pointerup', onUp);
    stage.addEventListener('pointercancel', onUp);
    return () => {
      stage.removeEventListener('pointerdown', onDown);
      stage.removeEventListener('pointermove', onMove);
      stage.removeEventListener('pointerup', onUp);
      stage.removeEventListener('pointercancel', onUp);
    };
  }, [stageRef, contentRef]);

  return { dragging };
}
