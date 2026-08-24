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
