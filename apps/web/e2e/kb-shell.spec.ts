import { test, expect } from '@playwright/test';
import { resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * The catalogue's fixed shell.
 *
 * `CatalogueLayout.jsx` opens by calling the shell "fixed and not a screen's
 * to vary" — counts, then categories as tabs, above every state. The port
 * shipped without it, and that is what made the catalogue read as unported
 * however much the browse grid underneath had changed: `/kb` was a list of
 * five links, and `/kb/[category]` had no way to reach another category.
 *
 * Asserted on both routes because the whole point is that it does not vary.
 */
test.beforeEach(async ({ request }) => {
  await resetScenario(request);
  await setScenario(request, scenarioFor('kb-shell'));
});

test('the catalogue header is on the landing and on a category', async ({
  page,
}) => {
  for (const url of ['/kb', '/kb/vehicle']) {
    await page.goto(url);
    const head = page.locator('.hp-cathead');
    await expect(head, url).toBeVisible();
    // Counts, one per live category. `SubStats` renders each as a
    // `<div><span>label</span><b>value</b></div>` inside `.hp-subs`.
    await expect(head.locator('.hp-subs > div'), url).not.toHaveCount(0);
    // Tabs, as links — not client state.
    const tabs = head.getByRole('navigation', { name: 'Catalogue categories' });
    await expect(tabs.getByRole('link'), url).not.toHaveCount(0);
  }
});

test('a category view marks the category you are in', async ({ page }) => {
  // The failure this catches is not "no tabs" but "tabs that never say where
  // you are" — which is how the old `/kb` list behaved.
  await page.goto('/kb/vehicle');
  const current = page.locator('.hp-cattab[aria-current="page"]');
  await expect(current).toHaveCount(1);
  await expect(current).toHaveText('Vehicles');
});

test('you can cross from one category to another without going back', async ({
  page,
}) => {
  await page.goto('/kb/vehicle');
  await page
    .getByRole('navigation', { name: 'Catalogue categories' })
    .getByRole('link', { name: 'Weapons' })
    .click();
  await expect(page).toHaveURL(/\/kb\/weapon$/);
  await expect(page.locator('.hp-cattab[aria-current="page"]')).toHaveText(
    'Weapons',
  );
});

test('a category with nothing behind it is not offered', async ({ page }) => {
  // Contracts are ingest-sourced, so the count is legitimately zero on a fresh
  // instance. Same rule as the lens rail: a destination with nothing in it is
  // not a destination.
  await page.goto('/kb');
  const tabs = page.getByRole('navigation', { name: 'Catalogue categories' });
  const labels = await tabs.getByRole('link').allTextContents();
  expect(labels).toContain('Vehicles');
  expect(labels).not.toContain('Contracts');
});
