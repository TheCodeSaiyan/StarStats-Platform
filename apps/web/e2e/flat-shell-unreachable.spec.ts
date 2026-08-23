/**
 * The flat shell has no reachable host.
 *
 * This replaces `theme-toggle.spec.ts` and `topbar-layout-fetches.spec.ts`,
 * which tested the flat `TopBar` — its theme dropdown and its two layout-level
 * fetches. Both moved host four times as the port advanced (`/settings` →
 * `/discover` → `/orgs` → `/settings/widget-sharing`) and each carried a note
 * saying they would retire when the last one ported. It has.
 *
 * What replaces them is not nothing. Deleting a spec because its subject moved
 * out of reach is only safe if something asserts it IS out of reach — otherwise
 * a future change that re-renders the flat chrome on one route brings back the
 * double-chrome bug with no test between it and a merge.
 *
 * `TopBar`, `LeftRail`, `AccountMenu`, `TelemetryTicker`, `DrawerScrim`,
 * `RoutePlacard`, `DrawerToggle`, `ThemeToggle` and both footers are now
 * DELETED, along with the four shell fetches that fed them on every signed-in
 * page. So the assertions below changed shape: "present but hidden" became
 * "absent", because asserting a deleted element is hidden passes on nothing —
 * a trap this suite has hit three times.
 */
import { test, expect } from '@playwright/test';
import {
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

const SIGNED_IN = [
  '/me',
  '/settings',
  '/settings/widget-sharing',
  '/sharing',
  '/discover',
  '/orgs',
  '/submissions',
  '/admin',
] as const;

const SIGNED_OUT = ['/', '/about', '/kb', '/downloads', '/auth/login'] as const;

/*
 * `test.slow()` for this file, and it is not a workaround.
 *
 * This spec exists to visit MANY routes and assert almost nothing about each —
 * it is a sweep, not a scenario. The config's 10s navigation budget is sized
 * for a warm route; several of these are the first request that route gets in
 * a run, so the dev server compiles them on demand and a cold compile can beat
 * it. That produced seven navigation timeouts once the file was split into one
 * test per route.
 *
 * The alternative was raising `navigationTimeout` globally, which would slow
 * every genuine hang in the suite to the same degree. This is the file that
 * needs the room.
 */
test.beforeEach(async ({ request, page }) => {
  test.slow();
  await resetScenario(request);
  await setScenario(request, scenarioFor('flat-shell', {
    'GET /v1/submissions': { status: 200, body: { submissions: [], has_more: false } },
    'GET /v1/admin/submissions/queue': { status: 200, body: { items: [], has_more: false } },
  }));
  await page.setViewportSize({ width: 1440, height: 900 });
});

/*
 * ONE TEST PER ROUTE, not one loop over eight.
 *
 * This was a single case walking every signed-in route, and it intermittently
 * blew the 30s budget under full-suite load — eight page loads plus their
 * assertions do not fit in one case's timeout on a dev server that is also
 * compiling. Raising the timeout would have hidden that; splitting gives each
 * route its own budget and names the failing one in the report.
 */
for (const route of SIGNED_IN) {
  test(`${route} shows no flat chrome`, async ({ page }) => {
    await loginAs(page, { handle: 'TestPilot', staffRoles: ['admin'] });
    await page.goto(route);
    await expect(page).toHaveURL(new RegExp(`${route}/?$`));
    // The volume is there…
    await expect(page.locator('.hp-stage')).toHaveCount(1);
    // …and the chrome it replaced is GONE, not hidden. These components were
    // deleted once nothing could reach them, so `toHaveCount(0)` is the honest
    // assertion and `toBeHidden()` would be the vacuous one.
    await expect(page.locator('.ss-topbar')).toHaveCount(0);
    await expect(page.locator('.ss-rail')).toHaveCount(0);
    await expect(page.locator('.ss-app-footer')).toHaveCount(0);
  });
}


/* Split for the same reason as the signed-in set above. */
for (const route of SIGNED_OUT) {
  test(`${route} shows no flat chrome`, async ({ page }) => {
    await page.goto(route);
    await expect(page.locator('.hp-stage')).toHaveCount(1);
    await expect(page.locator('.site-footer')).toHaveCount(0);
    await expect(page.locator('header.ss-marketing-nav')).toHaveCount(0);
  });
}


test('the calibration pips replace the flat theme toggle', async ({ page }) => {
  // The retired `theme-toggle.spec.ts` drove the flat TopBar's theme dropdown
  // and asserted `html[data-theme]`. Its successor is the chrome's calibration
  // pips, which set `data-cal` on the STAGE — never on `<html>`, so the beam
  // cannot leak onto anything that has not been ported.
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/settings');
  const stage = page.locator('.hp-stage');
  await expect(stage).toHaveAttribute('data-cal', 'terra');
  // Scoped to the CHROME's pips: `/settings` also renders a `CalibrationChoice`
  // in its own body, so an unscoped locator matches both.
  await page
    .locator('.hp-top')
    .getByRole('button', { name: /pyro calibration/i })
    .click();
  await expect(stage).toHaveAttribute('data-cal', 'pyro');
  await expect(page.locator('html')).not.toHaveAttribute('data-cal', /.*/);
});

test('the error and not-found boundaries are projections too', async ({
  page,
  request,
}) => {
  // These were the LAST place the flat shell was visible, and they are the
  // easiest to miss: a root `error.tsx` replaces everything below the root
  // layout but not the layout itself, so `.ss-projection-root` disappears and
  // the flat chrome un-hides. Every route being ported did not cover it.
  await setScenario(request, scenarioFor('boundary-404'));
  await page.goto('/no-such-page-anywhere');
  await expect(page.locator('.hp-stage')).toHaveCount(1);
  await expect(page.locator('.ss-topbar')).toHaveCount(0);
  await expect(page.locator('h1')).toHaveText('Page not found');

  // The error boundary needs a route that genuinely THROWS. The first version
  // of this used `/kb/vehicle` with a 500 — which soft-fails to an empty
  // catalogue, so the boundary never rendered and the assertion passed on the
  // ordinary page. The entity DETAIL route re-throws a non-404 (`throw new
  // Error` in its page), so it reaches `kb/error.tsx`.
  await setScenario(request, {
    __id: 'boundary-error',
    routes: {
      'GET /v1/reference/vehicle/slug/anything': { status: 500, body: {} },
    },
  });
  await page.goto('/kb/vehicle/anything');
  // Assert the BOUNDARY rendered, by its own copy — not merely that a stage
  // exists, which is true of the working page too.
  await expect(page.locator('h1')).toHaveText(
    'Couldn’t load the knowledge base',
  );
  await expect(page.locator('.hp-stage')).toHaveCount(1);
  await expect(page.locator('.ss-topbar')).toHaveCount(0);
});

test('chrome navigation is a client transition, not a page reload', async ({
  page,
}) => {
  // The flat chrome used `next/link`; the kit's `ChromeBar` renders plain
  // anchors, which are right for a router-agnostic design system and are a FULL
  // DOCUMENT LOAD in this app. After the port every nav and account click
  // reloaded the page — losing scroll, losing state, costing about a second —
  // and nothing failed, because a full load still lands on the right URL.
  //
  // A marker on `window` is the only honest probe: it survives a client
  // transition and cannot survive a reload.
  await loginAs(page, { handle: 'TestPilot' });
  await page.setViewportSize({ width: 1600, height: 900 });
  await page.goto('/settings');

  await page.evaluate(() => {
    (window as unknown as Record<string, unknown>).__transitionProbe = 'kept';
  });

  const toggle = page.locator('.hp-navtoggle');
  if (await toggle.isVisible()) await toggle.click();
  await page.locator('.hp-lk a', { hasText: 'Catalogue' }).first().click();
  await page.waitForURL(/\/kb/);

  const kept = await page.evaluate(
    () => (window as unknown as Record<string, unknown>).__transitionProbe,
  );
  expect(kept).toBe('kept');
});
