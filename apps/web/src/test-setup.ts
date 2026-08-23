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
