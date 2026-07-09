// Global vitest setup — runs before each test file.

// jsdom in opaque-origin mode does not expose a functional localStorage.
// Install an in-memory shim so tests can call localStorage.clear() / setItem() / getItem().
const store: Record<string, string> = {};
const localStorageMock: Storage = {
  get length() { return Object.keys(store).length; },
  key(index: number) { return Object.keys(store)[index] ?? null; },
  getItem(key: string) { return store[key] ?? null; },
  setItem(key: string, value: string) { store[key] = value; },
  removeItem(key: string) { delete store[key]; },
  clear() { for (const k of Object.keys(store)) delete store[k]; },
};

Object.defineProperty(window, 'localStorage', {
  value: localStorageMock,
  writable: true,
});
