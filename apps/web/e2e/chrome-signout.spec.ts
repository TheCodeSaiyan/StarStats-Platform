/**
 * Sign-out, the front page for a signed-in reader, and opening a log row.
 *
 * Three things the first real use of the projection surfaced: the account
 * menu had no way out (no surface passed the chrome a sign-out handler, so
 * the button never rendered), the root bounced every signed-in visit to
 * `/me` so "Overview" in the nav led nowhere, and the activity log had no
 * way into an event.
 */
import { test, expect } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

test.beforeEach(async ({ page, request }) => {
  await resetScenario(request);
  await setScenario(request, scenarioFor('chrome-signout'));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.setViewportSize({ width: 1440, height: 900 });
});

for (const path of ['/me', '/me/activity']) {
  test(`the account menu on ${path} has a working sign-out`, async ({ page }) => {
    await page.goto(path);
    await page.locator('.hp-acct .btn').click();
    const out = page.locator('.hp-acct .menu .out', { hasText: 'Sign out' });
    await expect(out).toBeVisible();
    await out.click();
    await expect(page).toHaveURL(/\/$/);
    const cookies = await page.context().cookies();
    expect(cookies.find((c) => c.name === 'starstats_session')).toBeUndefined();
  });
}

test('a signed-in reader can open the front page and is not bounced to /me', async ({ page }) => {
  await page.goto('/');
  await expect(page).toHaveURL(/\/$/);
  await expect(page.locator('.hp-acct')).toBeVisible();
  await expect(page.locator('.hp-signin')).toHaveCount(0);
});

test('a log row opens into the event', async ({ page }) => {
  await page.goto('/me/activity');
  const row = page.locator('details.hp-lg-x').first();
  await expect(row).toBeVisible();
  await expect(row).not.toHaveAttribute('open', '');
  await row.locator('summary').click();
  await expect(row).toHaveAttribute('open', '');
  await expect(row.locator('.hp-kv')).toContainText(/Source/);
});
