/**
 * The settings surface, in the projection.
 *
 * NOT a capture spec any more. This file began as scaffolding for the port —
 * a set of `goto` + `waitForTimeout` + `screenshot` cases whose only job was
 * producing images to judge, plus the fixtures they needed. Those 28 cases
 * asserted nothing, slept for half a second each, and are gone; what is left
 * are the assertions written alongside them, which are about behaviour and
 * outlive the port.
 */
import { test, expect, type Page } from '@playwright/test';
import {
  currentUser,
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';


const consoleErrors: string[] = [];

/** An account with 2FA off and an unverified RSI handle — the state with the
 *  most on screen (the ownership procedure and the 2FA explainer both show). */
const FIXTURES = {
  // An UNVERIFIED handle, so the ownership procedure renders rather than the
  // verified branch.
  'GET /v1/auth/me': {
    status: 200,
    body: { ...currentUser.body, rsi_verified: false, totp_enabled: false },
  },
  'GET /v1/me/preferences': {
    status: 200,
    body: { theme: 'terra', theme_wave_speed: 'normal', timezone: null },
  },
  'POST /v1/auth/rsi/start': {
    status: 200,
    body: {
      code: 'SS-VERIFY-7Q4K2M9X',
      expires_at: '2026-08-23T12:00:00Z',
    },
  },
};

test.beforeEach(async ({ page, request }) => {
  consoleErrors.length = 0;
  page.on('console', (m) => {
    if (m.type() === 'error') consoleErrors.push(m.text());
  });
  page.on('pageerror', (e) => consoleErrors.push(`pageerror: ${e.message}`));
  await resetScenario(request);
  await setScenario(request, scenarioFor('settings-projection', FIXTURES));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.setViewportSize({ width: 1440, height: 900 });
});

async function openGroup(page: Page, name: string): Promise<void> {
  await page.locator('.hp-lens button', { hasText: name }).click();
}

test('a fragment opens its group (the lens-rail cost)', async ({ page }) => {
  // This is the behaviour grouping put at risk: every server action redirects
  // to a fragment, and the target section is not mounted until its group is.
  await page.goto('/settings#security');
  await expect(page.locator('.hp-settings')).toBeVisible();
  await expect(
    page.getByRole('heading', { name: 'Two-factor authentication' }),
  ).toBeVisible();
});

test('picking a calibration repaints the beam in place', async ({ page }) => {
  // The regression this guards: `setCalibrationAction` deliberately does not
  // revalidate, so rendering the server prop meant the pips fired the shock
  // ring and scan wipe over a volume that stayed the old colour until the next
  // navigation. Asserting `data-cal` on the STAGE (not on <html>, which the
  // projection never sets) is what actually differs.
  await page.goto('/settings');
  await expect(page.locator('.hp-settings')).toBeVisible();
  await expect(page.locator('.hp-stage')).toHaveAttribute('data-cal', 'terra');

  await page.locator('.hp-calchoice button', { hasText: 'Pyro' }).click();

  await expect(page.locator('.hp-stage')).toHaveAttribute('data-cal', 'pyro');
  // ...and without a navigation.
  await expect(page).toHaveURL(/\/settings$/);
});

test('the page has exactly one h1, naming the page', async ({ page }) => {
  // Every flat screen these replaced had an h1; the projection has no titled
  // surface of its own, so it went missing on four ported pages at once
  // before a test noticed. The final crumb step carries it.
  await page.goto('/settings');
  await expect(page.locator('h1')).toHaveCount(1);
  await expect(page.locator('h1')).toHaveText('Calibrate');
});

test('no console errors on settings', async ({ page }) => {
  await page.goto('/settings');
  await expect(page.locator('.hp-settings')).toBeVisible();
  // Every group, because a group change mounts a whole different subtree —
  // the 2FA pane in particular. Checking only the landing group would miss it.
  for (const g of ['Account', 'Security', 'Danger', 'General']) {
    await openGroup(page, g);
    await page.waitForTimeout(250);
  }
  await page.waitForTimeout(1200);
  if (consoleErrors.length) {
    console.log(`CONSOLE ERRORS:\n${consoleErrors.join('\n---\n')}`);
  }
  expect(consoleErrors).toEqual([]);
});

test('settings states the retention window', async ({ page }) => {
  // `Calibrate.jsx` carries a Retention pane and this page had none, so the
  // single most important fact about a reader's data — that it is bounded at a
  // year — was surfaced nowhere in settings.
  //
  // The spec's other rows (storage bytes, free-tier cap, export format) are
  // deliberately absent: the product has no export endpoint and no storage
  // accounting, and inventing them on the page where a reader goes to find out
  // what is kept would be the worst possible place for a fabricated figure.
  await page.goto('/settings');
  await expect(page.locator('.hp-settings')).toBeVisible();
  const pane = page.locator('.hp-pane', { hasText: 'Retention' }).first();
  await expect(pane).toContainText('365 days');
  await expect(pane).toContainText('Deleted, not archived');
  // "All" means the retention limit, not all time — the vocabulary rule.
  await expect(pane).toContainText(/All rather than all time/i);
  // No invented figures.
  await expect(pane).not.toContainText(/free-tier|MB|NDJSON/i);
});
