/**
 * `/me/activity` — the full event log behind the projection's digest.
 *
 * Behavioural assertions only. The base scenario already serves
 * `GET /v1/me/events` (fifty `login` / `mission_complete` rows), which is
 * enough to prove the page renders sentences rather than identifiers and
 * that the filters are real links.
 */
import { test, expect } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

const consoleErrors: string[] = [];

test.beforeEach(async ({ page, request }) => {
  consoleErrors.length = 0;
  page.on('console', (m) => {
    if (m.type() !== 'error') return;
    consoleErrors.push(m.text());
  });
  page.on('pageerror', (e) => consoleErrors.push(`pageerror: ${e.message}`));
  await resetScenario(request);
  await setScenario(request, scenarioFor('activity-projection'));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.setViewportSize({ width: 1440, height: 900 });
});

test('renders the log as sentences, never the raw identifier', async ({ page }) => {
  await page.goto('/me/activity');
  await expect(page.getByRole('heading', { name: 'Activity' })).toBeVisible();
  // `mission_complete` has no curated label, so the fallback title-cases it;
  // the snake_case key must survive only as the tooltip.
  const rows = page.locator('.hp-lg');
  await expect(rows.first()).toBeVisible();
  await expect(page.locator('.hp-lg .ev', { hasText: 'mission_complete' })).toHaveCount(0);
  await expect(page.locator('.hp-lg .ev', { hasText: 'Mission Complete' }).first()).toBeVisible();
  expect(consoleErrors, consoleErrors.join('\n')).toEqual([]);
});

test('type chips filter through the URL', async ({ page }) => {
  await page.goto('/me/activity');
  // Scoped to the type strip: the records index above it is a chip strip too,
  // and its "Activity" entry is also `aria-current` on this page.
  const strip = page.locator('.hp-catstrip[aria-label="Event types"]');
  const chip = strip.locator('.hp-catchip', { hasText: 'Mission Complete' });
  await expect(chip).toHaveAttribute('href', /type=mission_complete/);
  await chip.click();
  await expect(page).toHaveURL(/type=mission_complete/);
  await expect(strip.locator('.hp-catchip[aria-current="page"]')).toHaveText(/Mission Complete/);
});

test('the digest on /me links here', async ({ page, request }) => {
  // `recent_activity` is opt-in on the layout, so the pane needs a
  // profile-layout fixture that enables it (see web-testing.md).
  await resetScenario(request);
  await setScenario(
    request,
    scenarioFor('activity-digest', {
      'GET /v1/users/me/profile-layout': {
        status: 200,
        body: { layout: [{ id: 'recent_activity', enabled: true, size: 'compact' }] },
      },
    }),
  );
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 40_000 });
  await expect(page.locator('.hp-lens').first()).toBeVisible({ timeout: 20_000 });
  await page.locator('.hp-lens button', { hasText: 'Activity' }).click();
  const pane = page.locator('.hp-plane', { hasText: 'Recent activity' });
  await expect(pane).toBeVisible({ timeout: 15_000 });
  await expect(pane.getByRole('link', { name: /see all/ })).toHaveAttribute('href', '/me/activity');
  // The digest, too, renders the label and not the identifier.
  await expect(pane.locator('.hp-lg .ev', { hasText: 'mission_complete' })).toHaveCount(0);
});
