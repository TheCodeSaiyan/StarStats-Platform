import '@testing-library/jest-dom/vitest';

/**
 * jsdom implements neither of these, and the projection chrome needs both:
 * `ChromeBar` measures whether its links fit (collapse is measured, never a
 * breakpoint — the link count is a consumer's choice), and `Projection` asks
 * whether the pointer is coarse before it enables parallax.
 *
 * Stubbed globally rather than per-test: every page test that renders a
 * projection surface hits them, and a per-file stub would be forgotten on the
 * next screen. The stubs are inert — `ResizeObserver` never fires, so the
 * chrome keeps its initial fit, and `matchMedia` reports "no match", which is
 * the fine-pointer default.
 */
if (typeof globalThis.ResizeObserver === 'undefined') {
  globalThis.ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  } as unknown as typeof ResizeObserver;
}

if (typeof window !== 'undefined' && !window.matchMedia) {
  window.matchMedia = ((query: string) => ({
    matches: false,
    media: query,
    onchange: null,
    addEventListener: () => {},
    removeEventListener: () => {},
    addListener: () => {},
    removeListener: () => {},
    dispatchEvent: () => false,
  })) as unknown as typeof window.matchMedia;
}

// jsdom implements neither `Element.prototype.scrollTo` nor `scrollIntoView`.
// `PaneSurface` calls `scrollTo` when a reader switches lens group, so without
// this the call throws INSIDE an event handler — which vitest reports as an
// unhandled error and which fails the run at the process level while every
// individual test still passes green. Assertions are unaffected either way:
// nothing in the suite asserts on scroll position, and a scroll reset is not
// behaviour a jsdom test can observe.
if (typeof Element !== 'undefined') {
  if (typeof Element.prototype.scrollTo !== 'function') {
    Element.prototype.scrollTo = () => {};
  }
  if (typeof Element.prototype.scrollIntoView !== 'function') {
    Element.prototype.scrollIntoView = () => {};
  }
}

/**
 * jsdom is configured here without a storage area, so `window.localStorage` is
 * `undefined` rather than an empty store — which is NOT the shape browsers
 * give you, and not the shape components guard for. Anything remembering a
 * per-viewer preference there — a collapsed section, a remembered tab — would
 * otherwise have its tests exercise the throw path only and never the real
 * one. (`EmitterPrompt` was the original reason; it has since been replaced
 * by the outstanding-task banner, which deliberately stores nothing.)
 *
 * Backed by a Map on `Storage.prototype`'s own methods so `vi.spyOn(
 * Storage.prototype, 'getItem')` still works for the blocked-storage tests.
 */
if (typeof window !== 'undefined' && !window.localStorage) {
  const store = new Map<string, string>();
  const area: Storage = {
    get length() {
      return store.size;
    },
    key: (i: number) => Array.from(store.keys())[i] ?? null,
    getItem: (k: string) => store.get(k) ?? null,
    setItem: (k: string, v: string) => void store.set(k, String(v)),
    removeItem: (k: string) => void store.delete(k),
    clear: () => store.clear(),
  };
  Object.setPrototypeOf(area, Storage.prototype);
  Object.defineProperty(window, 'localStorage', { value: area, configurable: true });
  Object.defineProperty(window, 'sessionStorage', { value: area, configurable: true });
}
