import { expect, test } from '@playwright/test';
import { resetScenario, scenarioFor, setScenario, unauthorized } from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

// The whole point: an anonymous visitor reaches help, NOT a login bounce.
test('support_is_help_not_login', async ({ page, request }) => {
  await setScenario(request, scenarioFor('support_anon', { 'GET /v1/auth/me': unauthorized }));
  await page.goto('/support');
  await expect(page).not.toHaveURL(/\/auth\/login/);
  await expect(page.getByRole('heading', { level: 1 })).toBeVisible();
  // Scope to <main> — the site nav also has a "Docs" link with the same
  // href, so an unscoped locator hits Playwright's strict-mode violation.
  await expect(
    page.getByRole('main').getByRole('link', { name: /docs/i }),
  ).toHaveAttribute('href', '/docs');
});

// And it is NOT silently redirected to the payment page.
test('support_not_redirected_to_donate', async ({ page, request }) => {
  await setScenario(request, scenarioFor('support_anon2', { 'GET /v1/auth/me': unauthorized }));
  await page.goto('/support');
  await expect(page).not.toHaveURL(/\/donate/);
});
