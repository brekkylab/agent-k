// jsdom ships no ResizeObserver, but components that measure element size to
// drive layout (e.g. the image preview's fit-width + drag-pan overflow checks)
// construct one on mount. Provide an inert stub so they can render under tests.
if (!('ResizeObserver' in globalThis)) {
  class ResizeObserverStub {
    observe(): void {}
    unobserve(): void {}
    disconnect(): void {}
  }
  (globalThis as { ResizeObserver: unknown }).ResizeObserver = ResizeObserverStub;
}
