import { expect, test } from '@playwright/test';
import { resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

// /docs is the last launch blocker: onboarding has four steps a stranger
// will not guess. These assert the pages RENDER and carry their heading —
// deliberately not their prose, which would only encode today's wording.
test('docs_quickstart_renders', async ({ page, request }) => {
  await setScenario(request, scenarioFor('docs_quickstart'));
  await page.goto('/docs');

  await expect(
    page.getByRole('heading', { name: /get starstats running/i, level: 1 }),
  ).toBeVisible();
});

test('docs_quickstart_has_step_anchors', async ({ page, request }) => {
  await setScenario(request, scenarioFor('docs_quickstart_anchors'));
  await page.goto('/docs');

  // The five steps a stranger will not guess. An anchor that goes missing
  // silently breaks every deep link we hand a stuck tester.
  for (const id of ['install', 'pair', 'cookie', 'verify', 'sync']) {
    await expect(page.locator(`#${id}`)).toBeAttached();
  }
});

test('docs_rsi_cookie_renders', async ({ page, request }) => {
  await setScenario(request, scenarioFor('docs_rsi_cookie'));
  await page.goto('/docs/rsi-cookie');

  await expect(
    page.getByRole('heading', { name: /the rsi cookie/i, level: 1 }),
  ).toBeVisible();
});

test('docs_troubleshooting_renders', async ({ page, request }) => {
  await setScenario(request, scenarioFor('docs_troubleshooting'));
  await page.goto('/docs/troubleshooting');

  await expect(
    page.getByRole('heading', { name: /when it isn.t working/i, level: 1 }),
  ).toBeVisible();
});
