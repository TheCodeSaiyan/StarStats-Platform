import { test, expect } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * The projection layout editor: what it claims, and what it delivers.
 *
 * THE FAULT. `callouts` and `planes` are built SERVER-side from the saved
 * layout, so the client can only filter the set it was handed — an id the
 * server has not seen has no data and no view model. Adding a widget
 * therefore changed the counter and nothing else. Measured before the fix:
 * the note moved from "1 of 22 projected" to "2 of 22 projected" while the
 * number of drawn planes stayed at zero. The editor reported success for
 * something it had not done, and a failed save looked identical to a good one
 * because `persist` was fire-and-forget.
 *
 * THE FIX has two halves and this file covers both: the editor separates what
 * is projected from what is still loading, and the save is followed by a
 * `router.refresh()` so the server rebuilds with the new layout and the
 * widget actually appears.
 *
 * THE MOCK DOES NOT PERSIST. A real backend answers the next GET with the
 * layout it was just sent; the fixture server does not, so the scenario is
 * swapped after the click to stand in for that. Without it the refresh
 * re-reads the ORIGINAL layout and no amount of correct client code could
 * draw the widget — the test would be measuring the harness.
 */
const BASE = {
  'GET /v1/users/me/profile-layout': {
    status: 200,
    body: { layout: [{ id: 'travel', enabled: true, size: 'compact' }] },
  },
  'GET /v1/me/stats/fleet': {
    status: 200,
    body: { ships: [{ vehicle_class: 'AEGS_Avenger_Stalker', trip_count: 12 }] },
  },
};

/** What the backend would return once the save has landed. */
const AFTER_SAVE = {
  ...BASE,
  'GET /v1/users/me/profile-layout': {
    status: 200,
    body: {
      layout: [
        { id: 'travel', enabled: true, size: 'compact' },
        { id: 'fleet', enabled: true, size: 'compact' },
      ],
    },
  },
};

async function openEditor(page: import('@playwright/test').Page) {
  await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 30_000 });
  await expect(page.locator('.hp-lens').first()).toBeVisible({ timeout: 20_000 });
  // `E` opens the layout editor. Retried: the shortcut is bound after
  // hydration, so a keypress can land on markup that is not listening yet.
  await expect(async () => {
    await page.keyboard.press('e');
    await expect(page.locator('.hp-layout')).toBeVisible({ timeout: 2500 });
  }).toPass({ timeout: 30_000 });
}

test.describe('projection layout', () => {
  test.beforeEach(async ({ request }) => {
    await resetScenario(request);
    await setScenario(request, scenarioFor('layout-editor', BASE));
  });

  test('an added widget is reported as loading, not as projected', async ({ page }) => {
    test.slow();
    await loginAs(page, { handle: 'TestPilot' });
    await page.setViewportSize({ width: 1600, height: 950 });
    await openEditor(page);

    const note = page.locator('.hp-layout .note');
    await expect(note).toContainText('1 of');

    await page.locator('.hp-el', { hasText: 'Ships' }).locator('button.add').first().click();

    // The count must NOT claim the new widget is projected — that is the
    // specific lie this fixes. It is loading until the server has it.
    await expect(note).toContainText('1 of');
    await expect(note).toContainText('loading');
    // And the reader can see WHICH row is waiting.
    await expect(page.locator('.hp-el[data-pending="true"]')).toHaveCount(1);
  });

  test('the widget draws once the server has the new layout', async ({ page, request }) => {
    test.slow();
    await loginAs(page, { handle: 'TestPilot' });
    await page.setViewportSize({ width: 1600, height: 950 });
    await openEditor(page);

    // A PLANE LIVES INSIDE A PANE, and panes only draw with a lens open —
    // in overview mode there are none at all. Asserting on `.hp-plane` from
    // the overview measures nothing, which is how the first draft of this
    // test "failed" against a working fix.
    const openTravel = async () => {
      const btn = page.locator('.hp-lens button', { hasText: 'Travel' });
      await expect(async () => {
        await btn.click();
        await expect(page.locator('.hp-pane').first()).toBeVisible({ timeout: 2500 });
      }).toPass({ timeout: 30_000 });
    };
    await openTravel();
    const ships = page.locator('.hp-plane', { hasText: 'Ships you fly' });
    await expect(ships).toHaveCount(0);

    // Back to the editor and add it.
    await expect(async () => {
      await page.keyboard.press('e');
      await expect(page.locator('.hp-layout')).toBeVisible({ timeout: 2500 });
    }).toPass({ timeout: 30_000 });

    // Swapped BEFORE the click, not after: the refresh fires the moment the
    // save resolves, so a swap afterwards races it and the refresh reads the
    // OLD layout. A real backend has already persisted by the time the
    // refresh's GET arrives, which is what this reproduces.
    await setScenario(request, scenarioFor('layout-editor', AFTER_SAVE));
    await page.locator('.hp-el', { hasText: 'Ships' }).locator('button.add').first().click();

    // The plane appears without the reader reloading — the whole point.
    // Before the fix it never appeared at all.
    await expect(ships).toHaveCount(1, { timeout: 25_000 });

    // And once it is drawn, nothing is left loading.
    await expect(page.locator('.hp-layout .note')).not.toContainText('loading');
    await expect(page.locator('.hp-el[data-pending="true"]')).toHaveCount(0);
  });
});
