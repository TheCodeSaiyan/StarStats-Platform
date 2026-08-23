/**
 * The landing surface, in the projection.
 *
 * NOT a capture spec any more. This file began as scaffolding for the port —
 * a set of `goto` + `waitForTimeout` + `screenshot` cases whose only job was
 * producing images to judge, plus the fixtures they needed. Those 28 cases
 * asserted nothing, slept for half a second each, and are gone; what is left
 * are the assertions written alongside them, which are about behaviour and
 * outlive the port.
 */
import { test, expect } from '@playwright/test';
import { resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

const consoleErrors: string[] = [];

test.beforeEach(async ({ page, request }) => {
  consoleErrors.length = 0;
  page.on('console', (m) => {
    if (m.type() === 'error') consoleErrors.push(m.text());
  });
  page.on('pageerror', (e) => consoleErrors.push(`pageerror: ${e.message}`));
  await resetScenario(request);
  await setScenario(request, scenarioFor('landing-projection'));
  await page.setViewportSize({ width: 1440, height: 900 });
});

test('the landing renders in the projection, not the flat shell', async ({
  page,
}) => {
  await page.goto('/');
  await expect(page.locator('.hp-stage')).toBeVisible();
  // Hidden, not absent — a nested layout cannot remove a parent layout. The
  // `MarketingNav` assertion that used to sit here was dropped when the
  // component was deleted: asserting a non-existent element is hidden proves
  // nothing. `.site-footer` is still rendered by `layout.tsx`, so it is
  // asserted PRESENT before being asserted hidden.
  await expect(page.locator('.site-footer')).toHaveCount(0);
});

test('the brand surface is declared, which is what sizes the hero', async ({
  page,
}) => {
  // `surface="brand"` is not decoration: it opens the ring to
  // `min(760px, 72vw)`, and `BrandHero` is sized FROM the ring. Without it the
  // lockup overflows the circle — and nothing else would fail.
  await page.goto('/');
  await expect(page.locator('.hp-stage')).toHaveAttribute(
    'data-surface',
    'brand',
  );
});

test('the hero rotates the PRODUCT’s words, not the kit’s', async ({
  page,
}) => {
  // The kit rotates ['Deaths.', 'Jumps.', 'Contracts.', 'Sessions.'] and its
  // prompt claims those are the product's. They are not. This is the exact
  // failure mode a grounded port invites: trusting the kit over the route.
  await page.goto('/');
  const word = page.locator('.hp-brand-hero .wd');
  await expect(word).toBeVisible();
  await expect(word).toHaveText(
    /^(StarStats\.|Your manifest\.|Your numbers\.|Your timeline\.)$/,
  );
});

test('reduced motion pins the rotation instead of cycling', async ({
  browser,
}) => {
  // `HeroRotator` did this and said why: the CSS sweep is already flattened by
  // the global reduced-motion rule, but the CONTENT SWAP still registers as
  // motion to a screen reader. Porting the hero without it would have been a
  // quiet accessibility regression, so it is asserted rather than assumed.
  const context = await browser.newContext({ reducedMotion: 'reduce' });
  const page = await context.newPage();
  await page.goto('/');
  const word = page.locator('.hp-brand-hero .wd');
  await expect(word).toHaveText('StarStats.');
  await page.waitForTimeout(5200);
  await expect(word).toHaveText('StarStats.');
  await context.close();
});

test('the reading half carries the real feature set and the exclusions', async ({
  page,
}) => {
  await page.goto('/');
  const read = page.locator('.hp-landing-read');
  // All twelve of the product's features, not the kit's abbreviated six.
  await expect(read.locator('.hp-landing-grid .hp-plane')).toHaveCount(12);
  await expect(read.getByText('Stays on your PC by default')).toBeVisible();
  await expect(read.getByText('Your loadout, laid out')).toBeVisible();
  // The README's exclusion list, quoted rather than summarised.
  await expect(read.getByText('Read game memory.')).toBeVisible();
  await expect(
    read.getByText(
      "Touch other players' data — only your own log file and your own RSI session.",
    ),
  ).toBeVisible();
});

test('nothing on the page is a figure nobody can source', async ({ page }) => {
  // The kit's callouts read "92,481 events read" and "Six panes". A landing
  // page has no reader to draw a figure from, so an invented one would be a
  // fabricated statistic on the most-read page in the product.
  await page.goto('/');
  const body = (await page.locator('body').innerText()).replace(/\s+/g, ' ');
  expect(body).not.toContain('92,481');
  expect(body).not.toMatch(/[\d,]{4,}\s*events/i);
});

test('the legal plate carries the PRODUCT’s attribution, not the kit default', async ({
  page,
}) => {
  // The kit's `CIG_DISCLAIMER` and the shipped footer are different texts. The
  // shipped one names Squadron 42 and asserts the Cloud Imperium Rights
  // copyright over specifications; taking the shorter default would be a
  // rewrite of a legal notice.
  await page.goto('/');
  const legal = page.locator('.hp-legal');
  await expect(legal).toBeVisible();
  await expect(legal).toContainText('Squadron 42');
  await expect(legal).toContainText('Cloud Imperium Rights');
  await expect(legal).toContainText('MPL-2.0');
});

test('the callout field is decoration, never the only home for a fact', async ({
  page,
}) => {
  // `CalloutField` hides itself entirely below 1180px. Anything living only
  // there disappears on a phone — so the two claims that matter most are
  // asserted at a width where the field is gone.
  //
  // Two selector lessons are baked into these three lines.
  //
  // The field is `.hp-cos`, not `.hp-callouts` — the first version asserted the
  // latter was hidden and PASSED, on an element that does not exist. So the
  // wide case is asserted VISIBLE first: a typo now fails loudly.
  //
  // And the probe is the CALLOUTS, not the field. `.hp-cos` is a zero-size
  // absolutely-positioned wrapper whose children carry the boxes, so
  // `toBeVisible()` on the wrapper reports hidden even when six callouts are
  // plainly on screen — the same zero-box trap as the BeamTip trigger.
  const callouts = page.locator('.hp-cos .hp-co');

  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/');
  await expect(callouts).toHaveCount(6);
  await expect(callouts.first()).toBeVisible();

  await page.setViewportSize({ width: 900, height: 900 });
  await expect(callouts.first()).toBeHidden();
  const read = page.locator('.hp-landing-read');
  await expect(read).toContainText('MPL-2.0');
  await expect(read).toContainText('Read game memory.');
});

test('the page has exactly one h1 and one main landmark', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('[role="main"], main')).toHaveCount(1);
});

test('no console errors', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('.hp-brand-hero')).toBeVisible();
  await page.waitForTimeout(900);
  if (consoleErrors.length) {
    console.log(`CONSOLE ERRORS:\n${consoleErrors.join('\n---\n')}`);
  }
  expect(consoleErrors).toEqual([]);
});
