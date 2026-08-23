/**
 * The emitter surface, in the projection.
 *
 * NOT a capture spec any more. This file began as scaffolding for the port —
 * a set of `goto` + `waitForTimeout` + `screenshot` cases whose only job was
 * producing images to judge, plus the fixtures they needed. Those 28 cases
 * asserted nothing, slept for half a second each, and are gone; what is left
 * are the assertions written alongside them, which are about behaviour and
 * outlive the port.
 */
import { test, expect, type Page } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

const consoleErrors: string[] = [];

const DEVICE_A = 'dev_0191f3aa0c7e7b2ea1c4d5e6f7a8b9c0';
const DEVICE_B = 'dev_0191f3bb1d8f8c3fb2d5e6f7a8b9c0d1';

const FIXTURES = {
  'GET /v1/auth/devices': {
    status: 200,
    body: {
      devices: [
        {
          id: DEVICE_A,
          label: 'RIG-01',
          created_at: '2026-06-01T10:00:00Z',
          last_seen_at: new Date(Date.now() - 60_000).toISOString(),
          sync_enabled: true,
        },
        {
          id: DEVICE_B,
          label: 'LAPTOP',
          created_at: '2026-07-14T18:30:00Z',
          last_seen_at: '2026-08-19T21:00:00Z',
          sync_enabled: false,
        },
      ],
    },
  },
  'GET /v1/me/ingest-history': {
    status: 200,
    body: {
      batches: [
        {
          seq: 3,
          batch_id: 'b_0191f3cc2e9a9d40c3e6f7a8b9c0d1e2',
          occurred_at: new Date(Date.now() - 3_600_000).toISOString(),
          game_build: '4.2.1-LIVE.9876543',
          total: 1284,
          accepted: 1201,
          duplicate: 80,
          rejected: 3,
        },
        {
          seq: 2,
          batch_id: 'b_0191f3dd3fab0e51d4f7a8b9c0d1e2f3',
          occurred_at: new Date(Date.now() - 90_000_000).toISOString(),
          game_build: '4.2.1-LIVE.9876543',
          total: 812,
          accepted: 812,
          duplicate: 0,
          rejected: 0,
        },
      ],
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
  await setScenario(request, scenarioFor('emitter-projection', FIXTURES));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.setViewportSize({ width: 1440, height: 900 });
});

async function openGroup(page: Page, name: string): Promise<void> {
  await page.locator('.hp-lens button', { hasText: name }).click();
}

test('the device tabs are real links that reselect server-side', async ({
  page,
}) => {
  // Device selection is a NAVIGATION, not client state: switching re-fetches
  // that device's ingest batches. If this ever became a client toggle the
  // batches would silently belong to the wrong device.
  await page.goto('/downloads');
  await openGroup(page, 'Uplinks');
  await page
    .locator('.hp-devtabs a', { hasText: 'LAPTOP' })
    .click();
  await expect(page).toHaveURL(new RegExp(`device=${DEVICE_B}`));
  await openGroup(page, 'Uplinks');
  await expect(
    page.locator('.hp-devtabs a[aria-current="page"]'),
  ).toHaveText(/LAPTOP/);
});

test('the page has exactly one h1, naming the page', async ({ page }) => {
  // Every flat screen these replaced had an h1; the projection has no titled
  // surface of its own, so it went missing on four ported pages at once
  // before a test noticed. The final crumb step carries it.
  await page.goto('/downloads');
  await expect(page.locator('h1')).toHaveCount(1);
  await expect(page.locator('h1')).toHaveText('Emitter');
});

test('no console errors across every group', async ({ page }) => {
  await page.goto('/downloads');
  await expect(page.locator('.hp-settings')).toBeVisible();
  for (const g of ['Uplinks', 'Pair', 'Get it']) {
    await openGroup(page, g);
    await page.waitForTimeout(250);
  }
  await page.waitForTimeout(900);
  if (consoleErrors.length) {
    console.log(`CONSOLE ERRORS:\n${consoleErrors.join('\n---\n')}`);
  }
  expect(consoleErrors).toEqual([]);
});

/**
 * The guards below exist because every assertion above passed on a COMPLETELY
 * BLANK surface. The lens rail rendered three correctly-labelled groups and not
 * one pane under any of them — the group constants were exported from a
 * `'use client'` module, so a server component read `undefined` off them — and
 * "the h1 is right", "`.hp-settings` is visible" and "no console errors" were
 * all true of that empty page. Assert on the CONTENT, not the frame.
 */
test('every group renders panes, not just a rail', async ({ page }) => {
  await page.goto('/downloads');
  await expect(page.locator('.hp-settings')).toBeVisible();

  for (const [group, expected] of [
    ['Get it', ['Emitter', 'After install']],
    ['Pair', ['Generate a pairing code', 'Paste this into the tray']],
    ['Uplinks', ['Paired devices (2)', 'Recent ingest batches']],
  ] as const) {
    await openGroup(page, group);
    await expect(page.locator('.hp-phd h2')).toHaveText([...expected]);
  }
});

test('the download half lists a build per platform', async ({ page }) => {
  await page.goto('/downloads');
  const table = page.locator('.hp-tbl');
  await expect(table).toBeVisible();
  // The ARTIFACT labels, not the platform names — "Windows" alone also matches
  // the artifact cell and the detected-system chip, and a strict-mode match on
  // an ambiguous string proves less than it looks like it does.
  await expect(
    table.getByRole('cell', { name: 'Windows Installer (.exe)' }),
  ).toBeVisible();
  await expect(table.getByRole('cell', { name: /AppImage/ })).toBeVisible();
  // No macOS build on the track, and the fixture reflects that — the row is
  // present and says so rather than the platform silently vanishing.
  await expect(
    table.getByText('No macOS build yet — it’s on the roadmap.'),
  ).toBeVisible();
});

test('a signed-out visitor gets the download half and no pairing labels', async ({
  page,
  context,
}) => {
  await context.clearCookies();
  await page.goto('/downloads');
  await expect(page.locator('.hp-settings')).toBeVisible();
  // The access rule is that a visitor must not see even the LABEL of something
  // they cannot open, so these must be absent from the rail — not merely
  // disabled or empty.
  await expect(page.locator('.hp-lens button', { hasText: 'Pair' })).toHaveCount(
    0,
  );
  await expect(
    page.locator('.hp-lens button', { hasText: 'Uplinks' }),
  ).toHaveCount(0);
  await expect(page.locator('#device-label')).toHaveCount(0);
  // …but the download half is fully there.
  await expect(page.locator('.hp-tbl')).toBeVisible();
});

test('/devices redirects to the Emitter, keeping its query', async ({
  page,
}) => {
  // `/devices` is referenced by the terms of service, two guides, the features
  // page and the fleet pane. It stays a working URL; dropping `?device=` would
  // silently strand anyone arriving on a deep link.
  await page.goto(`/devices?device=${DEVICE_B}`);
  await expect(page).toHaveURL(`/downloads?device=${DEVICE_B}`);
  // `.hp-settings` being visible does NOT mean the page is interactive — the
  // whole surface is server-rendered, so the rail is on screen before React
  // has attached to it, and a click that lands in that window is simply lost.
  // After a redirect the timing is variable enough that this failed only in a
  // full-suite run and passed in isolation, which reads like flake and is not.
  // `toPass` re-drives the switch until the state it should have produced is
  // actually there.
  await expect(async () => {
    await openGroup(page, 'Uplinks');
    await expect(
      page.locator('.hp-devtabs a[aria-current="page"]'),
    ).toHaveText(/LAPTOP/, { timeout: 2000 });
  }).toPass({ timeout: 15_000 });
});

test('the Emitter renders in the projection, not the flat shell', async ({
  page,
}) => {
  await page.goto('/downloads');
  await expect(page.locator('.hp-stage')).toBeVisible();
  // HIDDEN, not absent — a nested layout cannot remove a parent layout, so the
  // flat chrome is still in the DOM and is taken out with `display: none`.
  await expect(page.locator('.ss-topbar')).toHaveCount(0);
  await expect(page.locator('.ss-rail')).toHaveCount(0);
});
