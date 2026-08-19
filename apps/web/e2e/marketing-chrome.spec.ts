import { expect, test } from '@playwright/test';
import {
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

// Guards against regression to the pre-fix state where the marketing
// nav was inlined in `/` and `/features` and missing from every other
// signed-out route. If someone moves <MarketingNav /> out of
// layout.tsx's signed-out branch and back into per-page bodies, these
// assertions catch it before merge.
const SIGNED_OUT_ROUTES = [
  '/about',
  '/privacy',
  '/terms',
  '/trust',
  '/docs',
  '/docs/rsi-cookie',
  '/docs/troubleshooting',
] as const;

for (const route of SIGNED_OUT_ROUTES) {
  test(`marketing_nav_renders_on_${route.replace('/', '')}`, async ({
    page,
    request,
  }) => {
    await setScenario(request, scenarioFor(`marketing_chrome_${route}`));
    await page.goto(route);

    // Brand mark links to /. Two links in the chrome point at /
    // (the brand mark in the nav AND the "About" link in the footer's
    // §11 block), so scope to the marketing nav header.
    const nav = page.locator('header.ss-marketing-nav');
    await expect(nav).toBeVisible();
    await expect(nav.getByRole('link', { name: /starstats/i })).toHaveAttribute(
      'href',
      '/',
    );
    await expect(
      nav.getByRole('link', { name: /get started/i }),
    ).toHaveAttribute('href', '/auth/signup');
  });
}

// H5: at ≤640px the nav collapses behind a hamburger. The old
// `nav a:nth-child(1..3){display:none}` block hid Features / StarPlatform
// / Privacy even inside the OPEN dropdown, so those pages were unreachable
// on mobile. After removing it, opening the hamburger must reveal them.
test('mobile hamburger reveals Features / StarPlatform / Privacy (H5)', async ({
  page,
  request,
}) => {
  await setScenario(request, scenarioFor('marketing_chrome_mobile_nav'));
  await page.setViewportSize({ width: 640, height: 900 });
  await page.goto('/');

  const nav = page.locator('header.ss-marketing-nav');
  const toggle = nav.locator('.ss-mnav-toggle');
  await expect(toggle).toBeVisible();

  await toggle.click();

  await expect(
    nav.getByRole('link', { name: 'Features', exact: true }),
  ).toBeVisible();
  await expect(
    nav.getByRole('link', { name: 'StarPlatform', exact: true }),
  ).toBeVisible();
  await expect(
    nav.getByRole('link', { name: 'Privacy', exact: true }),
  ).toBeVisible();
});

// The nav row carries eleven items. Its min-content width is ~1185px on
// Windows and ~1187px on the Linux runner — text metrics are per-platform,
// so the drawer bound is padded to 1239/1240 rather than shaved to the
// measurement. An earlier cut at 1179/1180 passed locally and failed here
// by exactly 7px; these widths straddle the padded bound deliberately.
//
// These pages have no horizontal scrollbar, so an overflowing nav is
// INVISIBLE — `Get started` just leaves the screen. That shipped
// undetected for a long time.
for (const width of [375, 640, 1024, 1239, 1240, 1280, 1366]) {
  test(`marketing nav does not overflow the viewport at ${width}px`, async ({
    page,
    request,
  }) => {
    await setScenario(request, scenarioFor('marketing_chrome_mobile_nav'));
    await page.setViewportSize({ width, height: 900 });
    await page.goto('/');
    await expect(page.locator('header.ss-marketing-nav')).toBeVisible();

    const overflow = await page.evaluate(
      () => document.documentElement.scrollWidth -
            document.documentElement.clientWidth,
    );
    expect(overflow).toBeLessThanOrEqual(0);

    // Zero overflow is NOT sufficient on its own. The row sets
    // `flex-wrap: wrap`, so an un-collapsed nav spills onto a second line
    // instead of scrolling — which is exactly how the broken drawer hid
    // for so long. Assert the two states are mutually exclusive: if the
    // hamburger is showing, the inline row must actually be gone.
    const state = await page.evaluate(() => {
      const nav = document.querySelector('header.ss-marketing-nav');
      const links = nav.querySelector('nav.ss-mnav-links');
      const toggle = nav.querySelector('.ss-mnav-toggle');
      const cta = [...nav.querySelectorAll('a')].pop();
      return {
        toggleShown: getComputedStyle(toggle).display !== 'none',
        rowShown: getComputedStyle(links).display !== 'none',
        navHeight: Math.round(nav.getBoundingClientRect().height),
        ctaOnScreen: cta.getBoundingClientRect().right <=
                     document.documentElement.clientWidth,
      };
    });
    expect(state.toggleShown && state.rowShown).toBe(false);
    // A wrapped row roughly doubles the header. Catches the same bug from
    // the other side, in case the display rule is defeated again.
    expect(state.navHeight).toBeLessThan(100);
    if (state.rowShown) expect(state.ctaOnScreen).toBe(true);
  });
}
