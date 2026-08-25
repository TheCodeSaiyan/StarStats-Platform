import { test, expect, type Page } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * The page must scroll wherever the pointer happens to be.
 *
 * `.hp-pane` is a scroll container with `overscroll-behavior: contain` —
 * correct for the FLOATING pane, which holds a long log inside a fixed volume.
 * `.hp-pane--static` docks that same pane into page flow and originally reset
 * only its position, so every static surface shipped panes that swallowed the
 * wheel: pointing at one froze the page, moving the pointer off it un-froze
 * it. `contain` blocks scroll chaining even when the container has nothing of
 * its own to scroll, which is exactly the case here.
 *
 * MEASURED BY SCROLLING, not by reading computed style. A style assertion
 * passes on any value someone decides is correct; the reader's complaint was
 * "the page does not scroll", so that is the assertion.
 */
async function scrollDeltaOver(
  page: Page,
  selector: string,
): Promise<number> {
  const box = await page.locator(selector).first().boundingBox();
  if (!box) throw new Error(`no box for ${selector}`);
  await page.locator('.hp-settings').evaluate((e) => {
    e.scrollTop = 0;
  });
  await page.mouse.move(box.x + box.width / 2, box.y + Math.min(60, box.height / 2));
  await page.mouse.wheel(0, 600);
  await page.waitForTimeout(350);
  return page.locator('.hp-settings').evaluate((e) => e.scrollTop);
}

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
  await setScenario(request, scenarioFor('scroll'));
});

test('the wheel scrolls the page from over a docked pane', async ({ page }) => {
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/settings');
  await expect(page.locator('.hp-settings')).toBeVisible();

  // Only meaningful if the page has somewhere to scroll to.
  const scrollable = await page
    .locator('.hp-settings')
    .evaluate((e) => e.scrollHeight > e.clientHeight + 40);
  expect(scrollable, 'page must overflow for this test to mean anything').toBe(
    true,
  );

  const overPane = await scrollDeltaOver(page, '.hp-pane');
  expect(overPane, 'wheel over a pane must scroll the page').toBeGreaterThan(0);
});

test('a docked pane is not its own scroll container', async ({ page }) => {
  // The containment belongs to the floating pane. Docked, it truncates the
  // content into an inner scrollbar halfway down the page as well as eating
  // the wheel.
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/settings');
  await expect(page.locator('.hp-settings')).toBeVisible();

  const style = await page.locator('.hp-pane--static').first().evaluate((el) => {
    const cs = getComputedStyle(el);
    return {
      overscroll: cs.overscrollBehaviorY,
      maxHeight: cs.maxHeight,
    };
  });
  expect(style.overscroll).not.toBe('contain');
  expect(style.maxHeight).toBe('none');
});

test('every static surface scrolls from over its content', async ({ page }) => {
  // Same class, so the fix is shared — but these are the surfaces a reader
  // actually spends time scrolling, and a regression on one is a regression on
  // all of them.
  await loginAs(page, { handle: 'TestPilot' });
  for (const url of ['/kb/vehicle', '/sharing']) {
    await page.goto(url);
    await expect(page.locator('.hp-settings'), url).toBeVisible();
    const scrollable = await page
      .locator('.hp-settings')
      .evaluate((e) => e.scrollHeight > e.clientHeight + 40);
    if (!scrollable) continue;
    expect(await scrollDeltaOver(page, '.hp-pane'), url).toBeGreaterThan(0);
  }
});

test('no scroll container paints a bar for a few pixels', async ({ page, request }) => {
  /**
   * A SCROLLBAR FOR THREE PIXELS IS A SCROLLBAR NOBODY ASKED FOR.
   *
   * `.hp-graf` was a fixed `height: 96px` box holding a caption plus a 76px
   * chart. When `--fs-micro` moved 8.5px -> 10px in the contrast pass the
   * caption grew and the content became 99px — three pixels that the graf
   * could not scroll away, so they propagated to the PANE, and every lens
   * whose pane held a trace grew a vertical scrollbar for content that fits
   * on screen. Reported twice as "side bars where unneeded".
   *
   * THE THRESHOLD IS THE POINT. Real overflow is fine and expected — the All
   * lens genuinely runs 300px past its pane. What is not fine is a bar for an
   * amount a reader cannot see, which is always a sizing bug rather than a
   * scrolling need. 8px is comfortably below anything legible and comfortably
   * above sub-pixel rounding.
   *
   * Only axes that CAN paint a bar are counted: an axis set to `hidden` or
   * `clip` overflows silently by design.
   */
  // Panes only draw for widgets the reader has enabled.
  await setScenario(request, scenarioFor('scroll-trivial', {
    'GET /v1/users/me/profile-layout': {
      status: 200,
      body: {
        layout: ['travel', 'routes', 'fleet', 'lives', 'contracts', 'spend', 'sessions']
          .map((id) => ({ id, enabled: true, size: 'compact' })),
      },
    },
  }));
  await loginAs(page, { handle: 'TestPilot' });
  await page.setViewportSize({ width: 1440, height: 900 });

  /**
   * THE LENS MUST BE OPEN. `/me` with no lens selected draws no pane, and the
   * fault lives in the pane — the first draft of this test visited the bare
   * page, found nothing, and passed against the very CSS it was written to
   * catch. Every lens is walked because the overflow depends on what the pane
   * happens to contain.
   */
  const offenders: string[] = [];
  const targets: { url: string; lens?: string }[] = [
    { url: '/me', lens: 'Activity' },
    { url: '/me', lens: 'Travel' },
    { url: '/me', lens: 'Combat' },
    { url: '/me', lens: 'Commerce' },
    { url: '/me/travel' },
    { url: '/settings' },
    { url: '/kb' },
    { url: '/sharing' },
  ];
  for (const { url, lens } of targets) {
    await page.goto(url, { waitUntil: 'domcontentloaded', timeout: 30_000 });
    if (lens) {
      const btn = page.locator('.hp-lens button', { hasText: lens });
      await expect(btn).toBeVisible({ timeout: 20_000 });
      // Retried: the control is server-rendered, so a click can land before
      // React attaches and the lens never opens.
      await expect(async () => {
        await btn.click();
        await expect(page.locator('.hp-pane').first()).toBeVisible({ timeout: 2500 });
      }).toPass({ timeout: 30_000 });
    }
    await page.waitForTimeout(700);
    const found = await page.evaluate(() => {
      const out: string[] = [];
      document.querySelectorAll<HTMLElement>('*').forEach((el) => {
        const cs = getComputedStyle(el);
        const scrollsY = cs.overflowY === 'auto' || cs.overflowY === 'scroll';
        const scrollsX = cs.overflowX === 'auto' || cs.overflowX === 'scroll';
        const dy = el.scrollHeight - el.clientHeight;
        const dx = el.scrollWidth - el.clientWidth;
        const tag = `${el.tagName}.${String(el.className).slice(0, 26)}`;
        if (scrollsY && dy > 0 && dy <= 8) out.push(`${tag} scrolls ${dy}px vertically`);
        if (scrollsX && dx > 0 && dx <= 8) out.push(`${tag} scrolls ${dx}px horizontally`);
      });
      return [...new Set(out)];
    });
    for (const f of found) offenders.push(`${url}${lens ? ' [' + lens + ']' : ''} — ${f}`);
  }
  expect(offenders, offenders.join(String.fromCharCode(10))).toEqual([]);
});
