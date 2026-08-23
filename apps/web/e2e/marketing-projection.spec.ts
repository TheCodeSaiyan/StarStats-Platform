/**
 * The marketing surface, in the projection.
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


/**
 * Every route the shared shell now frames, with the h1 its body renders —
 * VERBATIM.
 *
 * These were first written as loose patterns guessed from the route name
 * (`/Docs/i` for `/docs`) and seven of the nine failed, because the product's
 * headings are prose: "Get StarStats running.", "A pilot's logbook. Nothing
 * more." A guessed pattern that happens to match would have been worse than a
 * failing one — it would assert nothing while looking like it asserted the
 * page's identity. Quoted exactly, they also pin the copy against a port that
 * reworded it.
 */
const PAGES = [
  { route: '/features', h1: 'Go from this…to this.' },
  { route: '/star-platform', h1: 'Command your org, live.' },
  { route: '/about', h1: "A pilot's logbook. Nothing more." },
  { route: '/lore', h1: 'Universe primer' },
  { route: '/trust', h1: 'No surprises. Check for yourself.' },
  { route: '/privacy', h1: 'Privacy Policy' },
  { route: '/terms', h1: 'Terms of Service' },
  { route: '/docs', h1: 'Get StarStats running.' },
  { route: '/guides', h1: 'Using StarStats.' },
] as const;

test.beforeEach(async ({ request, page }) => {
  await resetScenario(request);
  await setScenario(request, scenarioFor('marketing-projection'));
  await page.setViewportSize({ width: 1440, height: 900 });
});

for (const { route, h1 } of PAGES) {
  test(`${route} renders in the projection with its body intact`, async ({
    page,
  }) => {
    await page.goto(route);

    // The frame.
    await expect(page.locator('.hp-stage')).toBeVisible();
    await expect(page.locator('header.ss-marketing-nav')).toHaveCount(0);

    // Exactly one main landmark, supplied by the pane — not by the page. Two
    // would mean the `<main>` → `<div>` rewrap missed one; zero would mean the
    // shell is not wrapping this route at all.
    await expect(page.locator('[role="main"], main')).toHaveCount(1);
    await expect(page.locator('main')).toHaveCount(0);

    // Exactly one h1, from the PAGE's own body. The shell deliberately does not
    // pass `crumbHeading`, because each of these already has a heading that
    // names it better than a shared crumb could — passing it would have shipped
    // two h1s on all nine at once.
    await expect(page.locator('h1')).toHaveCount(1);
    await expect(page.locator('h1')).toHaveText(h1);

    // …and the body actually made it inside the pane, rather than the shell
    // rendering an empty one around nothing.
    await expect(page.locator('.hp-marketing')).not.toBeEmpty();
  });
}

test('the prose keeps a reading column rather than running the full volume', async ({
  page,
}) => {
  // globals.css caps a bare `<main>` at 720px, and these pages relied on it.
  // Their `<main>` is a `<div>` now, so that rule no longer applies and the
  // column has to be re-established by the bridge — without it the prose runs
  // the whole width of the stage on a wide screen and nothing errors.
  await page.goto('/privacy');
  const width = await page
    .locator('.hp-marketing')
    .evaluate((el) => el.getBoundingClientRect().width);
  expect(width).toBeLessThanOrEqual(800);
  expect(width).toBeGreaterThan(400);
});

test('a signed-in reader gets their own chrome, not a Sign in', async ({
  page,
}) => {
  // These routes are public but not signed-out-only: a reader following a
  // footer link to /privacy has a session, and the flat shell wrapped them in
  // `.ss-app` for exactly that case.
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.goto('/privacy');
  await expect(page.locator('.hp-acct')).toBeVisible();
  await expect(page.locator('.hp-signin')).toHaveCount(0);
  // The flat signed-in shell must be gone too, not just the marketing nav.
  await expect(page.locator('.ss-topbar')).toHaveCount(0);
});

test('a signed-out visitor gets a Sign in and no private labels', async ({
  page,
}) => {
  await page.goto('/privacy');
  await expect(page.locator('.hp-signin')).toBeVisible();
  await expect(page.locator('.hp-acct')).toHaveCount(0);
  await expect(
    page.locator('.hp-lk').getByText('Calibrate', { exact: true }),
  ).toHaveCount(0);
});

test('the main landmark is the page body, not the chrome', async ({ page }) => {
  // The landmark used to sit on the surface root, which contains `ChromeBar` —
  // so a screen-reader user jumping to main got the nav with it, and any test
  // scoping to `main` to EXCLUDE the nav quietly stopped meaning anything.
  // It now sits on `#hp-content`, which wraps the page body alone.
  await page.goto('/docs');
  const main = page.getByRole('main');
  await expect(main).toHaveCount(1);
  // The chrome's nav is a sibling of the landmark, not inside it.
  await expect(main.locator('.hp-lk')).toHaveCount(0);
  await expect(main.locator('.hp-top')).toHaveCount(0);
  // …and the page's own body is.
  await expect(main.locator('.hp-marketing')).toHaveCount(1);
});

/**
 * The docs index, from `Docs.jsx` — the third grounded screen.
 *
 * Its shape is a grouped row set naming every doc, guide and project page, so
 * a reader who lands on "RSI cookie" can see that Troubleshooting and Support
 * exist. The product splits that content across twelve routes and each one was
 * a dead end before this: you arrived, read, and left the way you came.
 */
const INDEXED = [
  '/docs',
  '/docs/rsi-cookie',
  '/docs/troubleshooting',
  '/guides',
  '/guides/dashboard',
  '/support',
  '/changelog',
  '/lore',
] as const;

for (const route of INDEXED) {
  test(`${route} carries the docs index, marking itself`, async ({ page }) => {
    await page.goto(route);
    const index = page.locator('.hp-docsindex');
    await expect(index).toHaveCount(1);
    // The spec's four groups, in its order.
    await expect(index.locator('.hp-docsindex__grp')).toHaveText([
      'Product',
      'Help',
      'Guides',
      'Project',
    ]);
    // Exactly one entry marks itself as where you are — the kit does this with
    // client state, which a real route cannot and should not.
    const current = index.locator('[aria-current="page"]');
    await expect(current).toHaveCount(1);
    await expect(current).toHaveAttribute('href', route);
  });
}

/*
 * "Every docs-index entry resolves" lived here and was fifteen sequential page
 * loads in one test — slow, and it timed out under full-suite load. It is a
 * FILESYSTEM fact, not a rendering one, so it moved to
 * `src/components/projection/DocsIndex.test.ts` where it is deterministic and
 * instant. Kept as a note so nobody re-adds the e2e version.
 */

