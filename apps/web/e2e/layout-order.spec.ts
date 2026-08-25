import { test, expect, type Page } from '@playwright/test';
import {
  getCalls,
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

/**
 * The projection layout editor: what it SHOWS has to be what it SAVES.
 *
 * The editor listed every element in catalogue order while the reorder
 * controls moved a separate, invisible array. Measured before this suite:
 * moving "Top routes" earlier wrote `[routes, travel, spend]` and left the
 * visible list reading `[Spending, Quantum transits, Top routes]` — the row
 * the reader clicked did not move, on a control whose entire purpose is to
 * move it. Order is not decoration here: the callout field draws the first
 * six and reports the rest as undrawn, so this is the control that decides
 * which six a reader sees.
 *
 * The fixture leans on Callouts, where catalogue order and layout order
 * disagree the most: `travel` is LAST in the catalogue and first in the
 * layout below, so a suite that passes on catalogue order cannot pass here.
 */

const LAYOUT = [
  { id: 'travel', enabled: true, size: 'compact' },
  { id: 'spend', enabled: true, size: 'compact' },
  { id: 'sessions', enabled: true, size: 'compact' },
];

async function openEditor(page: Page) {
  await page.setViewportSize({ width: 1600, height: 950 });
  await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 40_000 });
  await expect(page.locator('.hp-lens').first()).toBeVisible({ timeout: 20_000 });
  // The binding is on window and the pane mounts after hydration, so press
  // until it takes rather than racing a fixed wait.
  await expect(async () => {
    await page.keyboard.press('e');
    await expect(page.locator('.hp-layout')).toBeVisible({ timeout: 2500 });
  }).toPass({ timeout: 30_000 });
}

/** The Callouts group's enabled rows, top to bottom, as the reader sees them. */
function callouts(page: Page) {
  return page
    .locator('.hp-layout > div')
    .filter({ has: page.locator('.grp', { hasText: 'Callouts' }) })
    .locator('.hp-el[data-on="true"]');
}
const names = (page: Page) =>
  callouts(page).evaluateAll((els) =>
    els.map((e) => (e.querySelector('.nm') as HTMLElement)?.innerText),
  );

test.describe('projection layout order', () => {
  test('rows are listed in layout order, not catalogue order', async ({
    page,
    request,
  }) => {
    test.slow();
    await resetScenario(request);
    await setScenario(
      request,
      scenarioFor('lo', {
        'GET /v1/users/me/profile-layout': { status: 200, body: { layout: LAYOUT } },
      }),
    );
    await loginAs(page, { handle: 'TestPilot' });
    await openEditor(page);

    // Catalogue order would be Spending, Play sessions, Quantum transits.
    expect(await names(page)).toEqual([
      'Quantum transits',
      'Spending',
      'Play sessions',
    ]);
  });

  test('moving a row later moves it on screen and in what is saved', async ({
    page,
    request,
  }) => {
    test.slow();
    await resetScenario(request);
    await setScenario(
      request,
      scenarioFor('lo', {
        'GET /v1/users/me/profile-layout': { status: 200, body: { layout: LAYOUT } },
      }),
    );
    await loginAs(page, { handle: 'TestPilot' });
    await openEditor(page);

    await callouts(page)
      .filter({ hasText: 'Quantum transits' })
      .locator('button[aria-label$="later"]')
      .click();

    // The row the reader pressed has to be the row that moved.
    await expect
      .poll(() => names(page))
      .toEqual(['Spending', 'Quantum transits', 'Play sessions']);

    // The list moves optimistically, so the write is still in flight when the
    // assertion above passes — poll for it rather than reading once and
    // recording a zero that only means "not yet".
    await expect
      .poll(
        async () => {
          const puts = (await getCalls(request)).filter(
            (c) => c.method === 'PUT' && c.path.includes('profile-layout'),
          );
          if (puts.length === 0) return null;
          const body = puts[puts.length - 1].body as {
            layout: { id: string; enabled: boolean }[];
          };
          return body.layout
            .filter((e) => e.enabled)
            .map((e) => e.id)
            .slice(0, 3);
        },
        { timeout: 15_000 },
      )
      // Screen and payload agree — that agreement is the whole point.
      .toEqual(['spend', 'travel', 'sessions']);
  });

  test('both directions exist, and the ends do not offer a dead press', async ({
    page,
    request,
  }) => {
    test.slow();
    await resetScenario(request);
    await setScenario(
      request,
      scenarioFor('lo', {
        'GET /v1/users/me/profile-layout': { status: 200, body: { layout: LAYOUT } },
      }),
    );
    await loginAs(page, { handle: 'TestPilot' });
    await openEditor(page);

    // There was only ever an "earlier" control: an element could be promoted
    // and never demoted.
    const down = callouts(page).locator('button[aria-label$="later"]');
    expect(await down.count()).toBe(3);

    const first = callouts(page).first();
    const last = callouts(page).last();
    await expect(first.locator('button[aria-label$="earlier"]')).toBeDisabled();
    await expect(last.locator('button[aria-label$="later"]')).toBeDisabled();
    await expect(first.locator('button[aria-label$="later"]')).toBeEnabled();
  });

  test('a refused write is not reported as saved', async ({ page, request }) => {
    test.slow();
    await resetScenario(request);
    await setScenario(
      request,
      scenarioFor('lo', {
        'GET /v1/users/me/profile-layout': { status: 200, body: { layout: LAYOUT } },
        // The server caps a layout at 32 entries and 400s past it, among
        // other refusals. The editor used to say "saved to your account" for
        // every one of them, because the action's `{ok:false}` was dropped on
        // the floor by the caller.
        'PUT /v1/users/me/profile-layout': { status: 400, body: {} },
      }),
    );
    await loginAs(page, { handle: 'TestPilot' });
    await openEditor(page);

    await page
      .locator('.hp-el')
      .filter({ hasText: 'Orgs' })
      .locator('button.add')
      .click();

    await expect(page.locator('.hp-layout .note')).toContainText(/could not save/i, {
      timeout: 15_000,
    });
  });
});
