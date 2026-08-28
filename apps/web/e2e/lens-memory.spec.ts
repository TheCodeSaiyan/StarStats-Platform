import { test, expect } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * `/me` opens on the lens the reader last used.
 *
 * Overview is the landing state by design — the ring, the callouts and the
 * trace — but a reader who works in the lists had to re-open their lens every
 * visit, which is what the overnight review meant by "the first thing a reader
 * sees has no lists in it".
 *
 * The choice is resolved SERVER-side from a cookie, so the assertions below
 * check the FIRST paint. Reading it in the browser instead would paint
 * overview and then swap, and a test that waits would never see the flash.
 */
test.describe('remembered lens', () => {
  test.beforeEach(async ({ request }) => {
    await resetScenario(request);
    await setScenario(request, scenarioFor('lens-memory'));
  });

  test('a first visit lands on overview', async ({ page }) => {
    await loginAs(page, { handle: 'TestPilot' });
    await page.setViewportSize({ width: 1600, height: 950 });
    await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 40_000 });
    await expect(page.locator('.hp-stage')).toHaveAttribute('data-mode', 'overview', {
      timeout: 20_000,
    });
  });

  test('the chosen lens is where the next visit opens', async ({ page }) => {
    await loginAs(page, { handle: 'TestPilot' });
    await page.setViewportSize({ width: 1600, height: 950 });
    await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 40_000 });
    await expect(page.locator('.hp-lens button').first()).toBeVisible({ timeout: 20_000 });

    await page.locator('.hp-lens button', { hasText: 'Travel' }).click();
    await expect(page.locator('.hp-stage')).toHaveAttribute('data-mode', 'detail');

    await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 40_000 });
    // `domcontentloaded` + no wait: this is the first paint, which is the
    // whole point of resolving the lens on the server.
    await expect(
      page.locator('.hp-stage'),
      'the saved lens must be open on arrival, not after a client swap',
    ).toHaveAttribute('data-mode', 'detail', { timeout: 20_000 });
    await expect(page.locator('.hp-lens button[aria-pressed="true"]')).toHaveText(
      /travel/i,
    );
  });

  test('returning to overview is remembered too', async ({ page }) => {
    await loginAs(page, { handle: 'TestPilot' });
    await page.setViewportSize({ width: 1600, height: 950 });
    await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 40_000 });
    await expect(page.locator('.hp-lens button').first()).toBeVisible({ timeout: 20_000 });
    await page.locator('.hp-lens button', { hasText: 'Combat' }).click();
    await expect(page.locator('.hp-stage')).toHaveAttribute('data-mode', 'detail');

    // Esc walks one depth out, back to overview.
    await page.keyboard.press('Escape');
    await expect(page.locator('.hp-stage')).toHaveAttribute('data-mode', 'overview');

    await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 40_000 });
    await expect(
      page.locator('.hp-stage'),
      'abandoning a lens must not re-open it next visit',
    ).toHaveAttribute('data-mode', 'overview', { timeout: 20_000 });
  });
});
