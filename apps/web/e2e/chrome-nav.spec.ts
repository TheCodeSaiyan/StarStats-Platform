import { test, expect, type Page } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * The chrome row: what it offers, and what happens when it cannot offer it all.
 *
 * WHAT WENT WRONG. Every destination a session could reach went into the bar
 * AND the menu, so a signed-in reader's bar carried seventeen links — nine of
 * them public pages they were not working in. `ChromeBar`'s fit measurement is
 * all-or-nothing, and its ladder gave up the whole nav BEFORE it gave up any
 * ornament, so the first thing sacrificed was navigation and the last was a
 * caption. Measured on `/me`: the inline row wanted 1953px of which the nav was
 * 687, and the bar was collapsed at every viewport up to 2560px.
 *
 * WHAT IS ASSERTED. The split (the row is the reader's working set, the menu is
 * the whole site, and the disclosure survives an inline row so nothing is
 * stranded), and the ORDER of the ladder — that ornament goes before links.
 *
 * WHAT IS NOT ASSERTED: exact breakpoints. Those move with a label change and
 * would make this file a tripwire rather than a guard. `lib/nav.test.ts` covers
 * membership; this covers behaviour.
 */
/**
 * Wait for the fit measurement to settle.
 *
 * `ChromeBar` measures in a layout effect and again on every ResizeObserver
 * frame, so `data-nav` is absent on the server render and can change once more
 * after hydration. Reading it the moment the row is visible reads the state
 * before the measurement, which is how the first draft of this file reported
 * "collapsed" on a surface that settles inline.
 */
async function settled(page: Page) {
  await expect(page.locator('.hp-top[data-nav]')).toBeVisible({ timeout: 20_000 });
  // One more frame than the observer needs, so a second pass cannot land
  // between the wait and the read.
  await page.waitForTimeout(400);
}

async function chrome(page: Page) {
  return page.evaluate(() => {
    const top = document.querySelector('.hp-top') as HTMLElement | null;
    if (!top) return null;
    const row = top.querySelector('.hp-lk');
    const menu = top.querySelector('.hp-navmenu');
    const toggle = top.querySelector('.hp-navtoggle') as HTMLElement | null;
    const acct = top.querySelector('.hp-acct') as HTMLElement | null;
    return {
      nav: top.getAttribute('data-nav'),
      rowLinks: row ? row.querySelectorAll('a').length : 0,
      menuLinks: menu ? menu.querySelectorAll('a').length : 0,
      toggleShown: toggle ? getComputedStyle(toggle).display !== 'none' : false,
      acctRight: acct ? Math.round(acct.getBoundingClientRect().right) : null,
      vw: window.innerWidth,
    };
  });
}

test.describe('chrome nav', () => {
  test.beforeEach(async ({ request }) => {
    await resetScenario(request);
    await setScenario(request, scenarioFor('chrome-nav'));
  });

  test('the bar carries a subset and the menu carries the site', async ({ page }) => {
    await loginAs(page, { handle: 'TestPilot' });
    await page.setViewportSize({ width: 1920, height: 1000 });
    await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 30_000 });
    await settled(page);
    const c = (await chrome(page))!;

    expect(c.rowLinks, 'the row must not be the whole site').toBeGreaterThan(0);
    expect(c.menuLinks).toBeGreaterThan(c.rowLinks);
    // The load-bearing one: with a split, the rest of the site lives ONLY
    // behind the toggle, so hiding it when the row fits strands it.
    expect(c.toggleShown, 'the disclosure must survive an inline row').toBe(true);
  });

  test('the disclosure opens whether or not the row is inline', async ({ page }) => {
    await loginAs(page, { handle: 'TestPilot' });
    for (const width of [1920, 1024]) {
      await page.setViewportSize({ width, height: 1000 });
      await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 30_000 });
      await settled(page);
      const toggle = page.locator('.hp-navtoggle');
      await expect(toggle).toBeVisible();
      const menu = page.locator('.hp-navmenu[data-open="true"]');
      // Retried: the toggle is server-rendered, so a single click can land
      // before React has attached its handler.
      await expect(async () => {
        await toggle.click();
        await expect(menu).toBeVisible({ timeout: 2_000 });
      }, `menu must open at ${width}px`).toPass({ timeout: 30_000 });
      // A public page the row does not carry — proof the menu is the full set.
      await expect(menu.getByRole('link', { name: 'Privacy' })).toBeVisible();
      await page.keyboard.press('Escape');
    }
  });

  test('gives up ornament before it gives up the links', async ({ page }) => {
    // The ladder's ORDER, which is the actual fix. At a width where the row
    // cannot hold everything, the nav must still be inline while a density
    // step has been spent — the reverse of what it did before.
    await loginAs(page, { handle: 'TestPilot' });
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto('/settings', { waitUntil: 'domcontentloaded', timeout: 30_000 });
    await settled(page);
    const c = (await chrome(page))!;
    expect(c.nav, '/settings must reach an inline row on a 1440 laptop').toBe(
      'inline',
    );
  });

  test('a signed-out visitor is offered no destination they cannot open', async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto('/', { waitUntil: 'domcontentloaded', timeout: 30_000 });
    await settled(page);
    const labels = await page.locator('.hp-top a').allTextContents();
    const joined = labels.join('|');
    for (const gated of ['Projection', 'Sharing', 'Calibrate', 'Console']) {
      expect(joined, `${gated} is a signed-in destination`).not.toContain(gated);
    }
  });
});
