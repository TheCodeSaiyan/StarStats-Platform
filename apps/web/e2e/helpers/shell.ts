import type { Page, Locator } from '@playwright/test';

/**
 * Locators for the projection shell that exclude the PENDING shell.
 *
 * `PageSkeleton` (`src/components/shell/PageSkeleton.tsx`) frames a route's
 * `loading.tsx` fallback in a complete second projection, so while it is on
 * screen the document holds two of everything the shell owns: `.hp-stage`,
 * `.ss-projection-root`, `.hp-settings__inner`, `.hp-crumb h1`.
 *
 * Under React's streaming SSR the fallback stays in the DOM while the real
 * content arrives in a hidden div and is swapped in by an inline script, so
 * the two genuinely coexist for a moment. A bare `page.locator('.hp-stage')`
 * is therefore a strict-mode violation waiting for a slow render — which is
 * what it did on 2026-09-12 in `lens-memory.spec.ts`, a spec that asserts on
 * first paint deliberately and so looks exactly when the window is open.
 *
 * These resolve to the REAL shell. They still wait: if only the fallback is up,
 * the locator is empty until the page lands, which is the behaviour a test
 * wants — it is waiting for the page, not settling for the skeleton.
 */
export function liveStage(page: Page): Locator {
  return page.locator('.hp-stage:not([data-pending])');
}

/** The real shell's root, excluding the pending one. */
export function liveRoot(page: Page): Locator {
  return page.locator('.ss-projection-root:not(:has(.hp-stage[data-pending]))');
}

/** Scope any selector to the real shell — `liveIn(page, 'h1')`, say. */
export function liveIn(page: Page, selector: string): Locator {
  return liveStage(page).locator(selector);
}
