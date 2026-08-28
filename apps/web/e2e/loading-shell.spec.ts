import { test, expect } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * The loading fallback fills the viewport.
 *
 * `layout.tsx` renders `.ss-app`, which `starstats-tokens.css` still styles as
 * the flat era's 2-column shell (`220px 1fr` / `56px 1fr`). Nothing occupies
 * those tracks any more — TopBar and LeftRail are deleted — and the only thing
 * collapsing the grid is `body:has(.ss-projection-root)` in
 * projection-shell.css.
 *
 * A `loading.tsx` fallback renders with the PAGE ABSENT, so no
 * `.ss-projection-root` is in the document and that `:has()` stops matching.
 * The shell springs back to the legacy grid and the fallback lands in the
 * top-left cell, scrollbars and all, on an otherwise blank page. Measured at
 * 220x96 in a 1600x950 viewport before the fix.
 *
 * Exactly the failure `BoundaryShell` was written for on error / not-found;
 * loading was simply never given the same frame.
 *
 * MEASURE THE BOX, don't assert visibility. The skeleton is `visible` and
 * on-screen in the broken state — it is 14% of the viewport width in the
 * wrong corner. Only the geometry tells the two apart.
 */
test('a loading fallback fills the viewport instead of the legacy rail cell', async ({
  page,
  request,
}) => {
  test.slow();
  await resetScenario(request);

  // Warm the route first. On a cold `next dev` compile the stylesheet has not
  // applied at measure time, `.ss-app` is not yet a grid, and the fallback
  // measures full-width for the wrong reason — a false pass, seen on the
  // first run of every fresh dev server and on no run after it.
  await setScenario(request, scenarioFor('loading-shell-warm', {}));
  await loginAs(page, { handle: 'TestPilot' });
  await page.setViewportSize({ width: 1600, height: 950 });
  await page.goto('/sharing', { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await page.waitForTimeout(500);

  await setScenario(
    request,
    scenarioFor('loading-shell', {
      // Holds the server render open so the streamed fallback stays on
      // screen long enough to measure.
      'GET /v1/me/shares': { status: 200, body: { shares: [] }, delayMs: 6000 },
    }),
  );
  await page.goto('/sharing', { waitUntil: 'commit', timeout: 60_000 });

  // Wait for the fallback AND a live stylesheet. Measuring the moment the
  // busy marker attaches is too early: the CSS has sometimes not applied
  // yet, and an unstyled `.ss-app` is not a grid, so the box measures
  // full-width for the wrong reason.
  await page.waitForFunction(
    () =>
      !!document.querySelector('[aria-busy="true"]') &&
      getComputedStyle(document.documentElement)
        .getPropertyValue('--s7')
        .trim() !== '',
    undefined,
    { timeout: 20_000 },
  );

  const m = await page.evaluate(() => {
    const main = document.querySelector('.ss-main') as HTMLElement | null;
    const r = main?.getBoundingClientRect();
    return {
      // The busy marker is the fallback's own, and it is gone the moment
      // the page arrives. Deliberately NOT keyed on `.ss-projection-root`
      // being absent: the fix gives the fallback one, so that signal would
      // silently invert and the test would pass by never looking.
      caught: !!document.querySelector('[aria-busy="true"]'),
      // A token resolving proves the stylesheet is live, so a full-width
      // measurement means the CSS decided that and not that the CSS is
      // missing.
      styled:
        getComputedStyle(document.documentElement)
          .getPropertyValue('--s7')
          .trim() !== '',
      w: r ? Math.round(r.width) : 0,
      h: r ? Math.round(r.height) : 0,
      vw: window.innerWidth,
      vh: window.innerHeight,
    };
  });

  expect(m.caught, 'never caught the loading fallback on screen').toBe(true);
  // Geometry alone would also be satisfied by the unconditional collapse in
  // projection-shell.css, so pin the frame as well: the fallback renders
  // inside a projection column, not as a bare skeleton on a blank page.
  await expect(
    page.locator('.hp-settings__inner[aria-busy="true"]'),
  ).toBeAttached();
  expect(m.styled, 'stylesheet had not applied at measure time').toBe(true);
  expect(
    m.w,
    `loading fallback is ${m.w}x${m.h} in a ${m.vw}x${m.vh} viewport`,
  ).toBeGreaterThan(m.vw * 0.9);
  expect(m.h).toBeGreaterThan(m.vh * 0.5);
});

/**
 * The shell does not depend on the page having rendered.
 *
 * The test above passes on the frame alone: `PageSkeleton` mounts a
 * `.ss-projection-root`, so the old `body:has(...)` predicate matched and the
 * geometry came out right either way. This one covers the other half — the
 * unconditional collapse in projection-shell.css — because not every fallback
 * has a frame. `auth/loading.tsx` deliberately keeps its own `hp-authpage`
 * markup and mounts no projection root; under the `:has()` predicate it drew
 * in the 220px rail cell exactly as the others did.
 *
 * Rather than race a real auth fallback, this removes the class from a loaded
 * page, which reproduces the one condition that mattered: a document with no
 * `.ss-projection-root` in it.
 */
test('the shell fills the viewport with no projection root in the document', async ({
  page,
  request,
}) => {
  test.slow();
  await resetScenario(request);
  await setScenario(request, scenarioFor('loading-shell-noroot', {}));
  await loginAs(page, { handle: 'TestPilot' });
  await page.setViewportSize({ width: 1600, height: 950 });
  await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 60_000 });
  const m = await page.evaluate(() => {
    for (const el of [...document.querySelectorAll('.ss-projection-root')]) {
      el.classList.remove('ss-projection-root');
    }
    const main = document.querySelector('.ss-main') as HTMLElement | null;
    const r = main?.getBoundingClientRect();
    return {
      w: r ? Math.round(r.width) : 0,
      vw: window.innerWidth,
      display: getComputedStyle(document.querySelector('.ss-app') as Element)
        .display,
    };
  });

  expect(m.display, 'shell reverted to the flat grid').toBe('block');
  expect(m.w, `.ss-main is ${m.w} of ${m.vw}`).toBeGreaterThan(m.vw * 0.9);
});
