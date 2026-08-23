/**
 * The contracts surface, in the projection.
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

const FIXTURES = {
  'GET /v1/me/stats/contracts': {
    status: 200,
    body: {
      total: 9, completed: 5, failed: 2, abandoned: 1,
      in_progress: 1, withdrawn: 0, unknown: 0, completion_pct: 63,
      runs: [
        {
          mission_id: 'mission_bounty_vhrt_01',
          name: 'Bounty: Very High Risk Target',
          state: 'completed',
          closed_by: 'hud_complete',
          accepted_at: '2026-08-21T18:00:00Z',
          closed_at: '2026-08-21T18:42:00Z',
          steps_complete: 3,
          step_count: 3,
          connected_server: 'SHARD-4B',
          partial_history: false,
          steps: [
            { order: 1, state: 'completed', text: 'Travel to the marked area', objective_id: null },
            { order: 2, state: 'completed', text: 'Eliminate the target', objective_id: null },
            { order: 3, state: 'completed', text: 'Return to a landing zone', objective_id: null },
          ],
        },
        {
          mission_id: 'mission_cargo_haul_02',
          name: 'Cargo haul',
          state: 'closed',
          closed_by: 'session_gap',
          accepted_at: '2026-08-20T09:00:00Z',
          closed_at: '2026-08-20T11:30:00Z',
          steps_complete: 1,
          step_count: 4,
          connected_server: null,
          partial_history: true,
          steps: [
            { order: 1, state: 'completed', text: 'Collect the cargo', objective_id: null },
            { order: 2, state: 'unknown', text: null, objective_id: 'obj_deliver_crate_a' },
          ],
        },
      ],
    },
  },
};

test.beforeEach(async ({ page, request }) => {
  consoleErrors.length = 0;
  page.on('console', (m) => {
    if (m.type() !== 'error') return;
    // Pre-existing mock gap: `/api/contracts/resolve` is not on the `/v1`
    // prefix the mock keys on, and the widget degrades correctly.
    if (m.text().includes('contracts resolve fetch failed')) return;
    consoleErrors.push(m.text());
  });
  page.on('pageerror', (e) => consoleErrors.push(`pageerror: ${e.message}`));
  await resetScenario(request);
  await setScenario(request, scenarioFor('contracts-projection', FIXTURES));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.setViewportSize({ width: 1440, height: 900 });
});

async function openGroup(page: Page, name: string): Promise<void> {
  await page.locator('.hp-lens button', { hasText: name }).click();
}

test('outcome wording comes from closed_by, not state', async ({ page }) => {
  // The distinction is the point of `_lib/outcome`: an observed HUD banner
  // versus a run inferred closed from a stream that went dead. Collapsing them
  // would report a guess as a fact.
  //
  // Asserted on the outcome CHIP, not on the plane. The first version of this
  // matched /completed/i anywhere in the plane — which the STEP labels also
  // say — so it passed while both chips read "no outcome recorded". A guard
  // aimed at the wrong element passes while the bug is present.
  await page.goto('/me/contracts');
  await openGroup(page, 'Runs');
  const chip = (name: string) =>
    page.locator('.hp-plane').filter({ hasText: name }).locator('.hp-chip');

  // Observed: the game showed a completion banner.
  await expect(chip('Bounty')).toHaveText('completed');
  // Inferred: `state: 'closed'` but closed by a session gap. It must read as
  // an inference, never as a plain completion.
  await expect(chip('Cargo haul')).toHaveText('abandoned — session gap');
});

test('an unresolved step falls back to its objective id, verbatim', async ({
  page,
}) => {
  // Engine ids are log literals and are never prettified.
  await page.goto('/me/contracts');
  await openGroup(page, 'Runs');
  await expect(page.getByText('obj_deliver_crate_a')).toBeVisible();
});

test('partial history is surfaced, not hidden', async ({ page }) => {
  await page.goto('/me/contracts');
  await openGroup(page, 'Runs');
  await expect(page.getByText(/history incomplete/i)).toBeVisible();
});

test('the page has exactly one h1, naming the page', async ({ page }) => {
  await page.goto('/me/contracts');
  await expect(page.locator('h1')).toHaveCount(1);
  await expect(page.locator('h1')).toHaveText('Contracts');
});

test('no console errors across every group', async ({ page }) => {
  await page.goto('/me/contracts');
  await expect(page.locator('.hp-settings')).toBeVisible();
  for (const g of ['Runs', 'Outcomes']) {
    await openGroup(page, g);
    await page.waitForTimeout(250);
  }
  await page.waitForTimeout(900);
  if (consoleErrors.length) {
    console.log(`CONSOLE ERRORS:\n${consoleErrors.join('\n---\n')}`);
  }
  expect(consoleErrors).toEqual([]);
});
