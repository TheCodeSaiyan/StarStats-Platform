import { test, expect } from '@playwright/test';
import { loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * "What you kill with" groups kills by maker instead of ranking flat.
 *
 * A weapon class carries manufacturer, family and size (`KLWE_LaserCannon_S2`),
 * so a flat top-N repeats the maker on every row and hides the fact that most
 * of your kills came out of one house.
 *
 * THE LOAD-BEARING ASSERTION IS THE PARENT TOTAL. "Does Klaus & Werner appear
 * somewhere" would also pass on a flat list, and a component that rendered an
 * empty tree would still typecheck, lint and clear the idiom sweep — that is
 * exactly how an earlier attempt at wiring this component up shipped drawing
 * nothing. 229 exists nowhere in the fixture; only a real roll-up can produce
 * it.
 */
test('the weapons pane rolls kills up by maker', async ({ page, request }) => {
  test.slow();
  await resetScenario(request);
  await setScenario(
    request,
    scenarioFor('weapon-rollup', {
      'GET /v1/me/stats/combat': {
        status: 200,
        body: {
          kills: 317,
          deaths: 12,
          incapacitated: 3,
          top_weapons: [
            { value: 'KLWE_LaserCannon_S2', count: 154 },
            { value: 'KLWE_LaserRepeater_S1', count: 75 },
            { value: 'BEHR_P4AR', count: 88 },
          ],
          deaths_by_zone: [],
        },
      },
      // The widget bails to null without a breakdown, and the objectives call
      // sits in the same allSettled, so both need fixtures or nothing renders.
      'GET /v1/me/metrics/event-types': {
        status: 200,
        body: {
          types: [
            { event_type: 'player_death', count: 34 },
            { event_type: 'vehicle_destruction', count: 22 },
            { event_type: 'mission_start', count: 48 },
            { event_type: 'mission_end', count: 41 },
          ],
        },
      },
      'GET /v1/me/stats/objectives': {
        status: 200,
        body: {
          completed: 84,
          failed: 9,
          unresolved: 7,
          no_outcome: 0,
          by_objective: [],
          lifetime: null,
          previous: null,
        },
      },
      'GET /v1/users/me/profile-layout': {
        status: 200,
        body: {
          layout: [{ id: 'combat_mission', enabled: true, size: 'compact' }],
        },
      },
    }),
  );
  await loginAs(page, { handle: 'TestPilot' });
  await page.setViewportSize({ width: 1600, height: 950 });
  await page.goto('/me', { waitUntil: 'domcontentloaded', timeout: 60_000 });
  await expect(page.locator('.hp-lens button').first()).toBeVisible({
    timeout: 30_000,
  });

  await page.locator('.hp-lens button', { hasText: 'Combat' }).click();
  const pane = page.locator('.hp-plane', { hasText: 'What you kill with' });
  await expect(pane).toBeVisible({ timeout: 20_000 });

  const collapsed = (await pane.innerText()).replace(/\s+/g, ' ');

  // 154 + 75 summed under one maker. That number is in no fixture field.
  expect(collapsed, `maker total missing from: ${collapsed}`).toContain('229');
  expect(collapsed).toContain('Klaus & Werner');
  // A second maker stands beside it, so this is a tree rather than one bucket.
  expect(collapsed).toContain('Behring');
  // The leaves are folded away until asked for.
  expect(collapsed).not.toContain('154');

  // The tree is native <details>/<summary>, so a closed node keeps its
  // children out of innerText entirely.
  await pane.locator('summary', { hasText: 'Klaus & Werner' }).click();
  const expanded = (await pane.innerText()).replace(/\s+/g, ' ');
  expect(expanded, `children missing from: ${expanded}`).toContain('154');
  expect(expanded).toContain('75');
});
