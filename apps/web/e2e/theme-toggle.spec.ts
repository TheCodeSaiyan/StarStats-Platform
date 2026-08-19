import { expect, test } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

test.describe('theme toggle', () => {
  test.beforeEach(async ({ request, page }) => {
    await resetScenario(request);
    await loginAs(page);
  });

  test('picking a theme from the TopBar toggle updates data-theme', async ({
    page,
    request,
  }) => {
    await setScenario(request, scenarioFor('theme_toggle'));
    await page.goto('/me');

    await page.getByRole('button', { name: /change theme/i }).click();
    await page.getByRole('menuitemradio', { name: /Pyro/i }).click();

    await expect(page.locator('html')).toHaveAttribute('data-theme', 'pyro');
  });

  test('still swaps under prefers-reduced-motion', async ({ browser, request }) => {
    await setScenario(request, scenarioFor('theme_toggle'));
    const context = await browser.newContext({ reducedMotion: 'reduce' });
    const page = await context.newPage();
    await loginAs(page);
    await page.goto('/me');

    await page.getByRole('button', { name: /change theme/i }).click();
    await page.getByRole('menuitemradio', { name: /Terra/i }).click();

    await expect(page.locator('html')).toHaveAttribute('data-theme', 'terra');
    await context.close();
  });
});
