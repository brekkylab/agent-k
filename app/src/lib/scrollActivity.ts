// Adds `.is-scrolling` to any `.cw-scroll-quiet` element while it's
// actively receiving scroll events, removing it after a brief idle so
// the CSS transition can fade the thumb back to invisible. Mirrors the
// macOS native scrollbar behavior — visible during activity or hover,
// quiet at rest — without forcing a permanent thumb that adds noise.

const SCROLL_FADE_MS = 800;
const timers = new WeakMap<Element, number>();

let installed = false;

export function installScrollActivity(): void {
  if (installed) return;
  installed = true;

  // Scroll events don't bubble; capture phase is the only way to delegate
  // from document. Passive because we never preventDefault here.
  document.addEventListener(
    'scroll',
    (event) => {
      const el = event.target as Element | null;
      if (!el || !(el as HTMLElement).classList?.contains?.('cw-scroll-quiet')) {
        return;
      }
      el.classList.add('is-scrolling');
      const prev = timers.get(el);
      if (prev) window.clearTimeout(prev);
      timers.set(
        el,
        window.setTimeout(() => {
          el.classList.remove('is-scrolling');
          timers.delete(el);
        }, SCROLL_FADE_MS),
      );
    },
    { capture: true, passive: true },
  );
}
