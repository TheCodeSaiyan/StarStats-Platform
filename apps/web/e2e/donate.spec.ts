import { expect, test } from '@playwright/test';
import { resetScenario, scenarioFor, setScenario, unauthorized } from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

// Donating still requires an account — anonymous visitor bounced to login
// (unchanged behaviour, just at the new /donate route).
test('donate_requires_login', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('donate_anon', { 'GET /v1/auth/me': unauthorized })
  );
  await page.goto('/donate');
  await expect(page).toHaveURL(/\/auth\/login/);
});
