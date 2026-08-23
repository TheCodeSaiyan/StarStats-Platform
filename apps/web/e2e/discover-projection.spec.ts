/**
 * The discover surface, in the projection.
 *
 * NOT a capture spec any more. This file began as scaffolding for the port —
 * a set of `goto` + `waitForTimeout` + `screenshot` cases whose only job was
 * producing images to judge, plus the fixtures they needed. Those 28 cases
 * asserted nothing, slept for half a second each, and are gone; what is left
 * are the assertions written alongside them, which are about behaviour and
 * outlive the port.
 */
import { test, expect } from '@playwright/test';
import {
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

const consoleErrors: string[] = [];

const PROFILES = {
  'GET /v1/discover/profiles': {
    status: 200,
    body: {
      profiles: [
        {
          handle: 'Alice',
          display_name: 'Alice Aviatrix',
          joined_at: '2026-01-01T00:00:00+00:00',
          last_active_at: '2026-08-20T00:00:00+00:00',
          supporter: 'active',
        },
        {
          handle: 'Bob',
          display_name: null,
          joined_at: '2026-02-01T00:00:00+00:00',
          last_active_at: null,
        },
        {
          handle: 'Cass',
          display_name: 'Cassiopeia',
          joined_at: '2026-03-01T00:00:00+00:00',
          last_active_at: '2026-07-02T00:00:00+00:00',
        },
      ],
      next_after: null,
    },
  },
};

test.beforeEach(async ({ page, request }) => {
  consoleErrors.length = 0;
  page.on('console', (m) => {
    if (m.type() === 'error') consoleErrors.push(m.text());
  });
  page.on('pageerror', (e) => consoleErrors.push(`pageerror: ${e.message}`));
  await resetScenario(request);
  await setScenario(request, scenarioFor('discover-projection', PROFILES));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.setViewportSize({ width: 1440, height: 900 });
});

test('the directory renders in the projection, not the flat shell', async ({
  page,
}) => {
  await page.goto('/discover');
  await expect(page.locator('.hp-stage')).toBeVisible();
  // HIDDEN, not absent. A nested layout cannot remove a parent layout in the
  // App Router, so the flat chrome is still in the DOM and `projection-shell
  // .css` takes it out with `display: none` — which is what also removes it
  // from the accessibility tree and the tab order. `toHaveCount(0)` would be
  // asserting a mechanism the port does not use, and it passes on a signed-OUT
  // page (no `.ss-app` at all) for a reason that has nothing to do with the
  // projection.
  await expect(page.locator('.ss-topbar')).toHaveCount(0);
  await expect(page.locator('.ss-rail')).toHaveCount(0);
});

test('the pane renders entries, not just a frame', async ({ page }) => {
  await page.goto('/discover');
  // The pane header by name, then the entries inside it. A visible surface
  // with an empty pane satisfies neither.
  await expect(page.locator('.hp-phd h2')).toHaveText(['Directory']);
  await expect(page.locator('.hp-dirrow')).toHaveCount(3);
  await expect(page.locator('.hp-dirrow__handle').first()).toHaveText('Alice');
});

test('one entry has no display name and does not invent one', async ({
  page,
}) => {
  await page.goto('/discover');
  const bob = page.locator('.hp-dirrow[data-handle="Bob"]');
  await expect(bob).toBeVisible();
  await expect(bob.locator('.hp-dirrow__name')).toHaveCount(0);
  // …and no last-active timestamp either. Missing is absent, never a zero or
  // an invented "unknown".
  await expect(bob.locator('.hp-dirrow .vv')).toHaveCount(0);
});

test('the page has exactly one h1, naming the page', async ({ page }) => {
  await page.goto('/discover');
  await expect(page.locator('h1')).toHaveCount(1);
  await expect(page.locator('h1')).toHaveText('Directory');
});

test('a single group hides the lens rail', async ({ page }) => {
  // A rail with one lit item reads as a control that does not work.
  await page.goto('/discover');
  await expect(page.locator('.hp-settings')).toBeVisible();
  await expect(page.locator('.hp-lens')).toHaveCount(0);
});

test('a signed-out visitor gets the listing and a Sign in', async ({
  page,
  context,
}) => {
  // The listing endpoint is unauthenticated by design and the flat page never
  // asked for a session — the port reads one only to pick the chrome.
  await context.clearCookies();
  await page.goto('/discover');
  await expect(page.locator('.hp-dirrow')).toHaveCount(3);
  await expect(page.locator('.hp-signin')).toBeVisible();
  await expect(page.locator('.hp-acct')).toHaveCount(0);
});

test('no console errors', async ({ page }) => {
  await page.goto('/discover');
  await expect(page.locator('.hp-settings')).toBeVisible();
  await page.waitForTimeout(900);
  if (consoleErrors.length) {
    console.log(`CONSOLE ERRORS:\n${consoleErrors.join('\n---\n')}`);
  }
  expect(consoleErrors).toEqual([]);
});
