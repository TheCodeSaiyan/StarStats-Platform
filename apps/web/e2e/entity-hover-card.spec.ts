import { expect, test } from '@playwright/test';
import { loginAs, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * The `EntityLink` hover card had the same defect InfoTip had (see
 * `infotip.spec.ts`): it opened, it was `visible`, and every scrolling
 * ancestor of the widget tile cut it off. A visibility assertion passes
 * on the broken code, and jsdom has no layout. This spec asserts what
 * actually differs: no ancestor of the card clips it, and it lands
 * inside the viewport.
 *
 * Hosted on `/u/[handle]`, which kept the flat `WidgetCanvas` and its
 * `.hud-tile` (overflow:hidden) + `.hud-tile__body` (overflow-y:auto).
 * The fleet widget links every ship through `EntityLink`, so it is the
 * natural anchor. The vehicle catalogue comes from the committed
 * `reference-data` snapshot, not a mocked endpoint, so the ship below
 * must be one the snapshot knows (`AEGS_Avenger_Stalker` →
 * `/kb/vehicle/avenger-stalker`, manufacturer Aegis Dynamics).
 */
const FIXTURES = {
  'GET /v1/users/me/profile-layout': {
    status: 200,
    body: { layout: [{ id: 'fleet', enabled: true, size: 'compact' }] },
  },
  'GET /v1/me/stats/fleet': {
    status: 200,
    body: {
      ships: [
        { vehicle_class: 'RSI_Aurora_MR', trip_count: 12 },
        { vehicle_class: 'AEGS_Avenger_Stalker', trip_count: 5 },
      ],
    },
  },
};

const LINK = '.hud-tile a[href="/kb/vehicle/avenger-stalker"]';

test('entity hover card is not clipped by its widget tile', async ({ page, request }) => {
  await loginAs(page, { handle: 'TestPilot' });
  await setScenario(request, scenarioFor('entity_hover_unclipped', FIXTURES));
  await page.goto('/u/TestPilot');

  // The last row: its card opens below it, past the tile body's bottom
  // edge, which is exactly where the old absolute card got cut off.
  const link = page.locator(LINK);
  await expect(link).toBeVisible();
  await link.hover();

  const card = page.getByRole('tooltip', { name: /avenger stalker details/i });
  await expect(card).toBeVisible();
  await expect(card).toContainText('Aegis Dynamics');

  const report = await page.evaluate(() => {
    const el = document.querySelector(
      '[role="tooltip"][aria-label="Avenger Stalker details"]',
    ) as HTMLElement;
    const r = el.getBoundingClientRect();
    const clippers: string[] = [];
    let a = el.parentElement;
    while (a && a !== document.documentElement) {
      const s = getComputedStyle(a);
      if (s.overflow !== 'visible' || s.overflowX !== 'visible' || s.overflowY !== 'visible') {
        const ar = a.getBoundingClientRect();
        if (r.top < ar.top || r.bottom > ar.bottom || r.left < ar.left || r.right > ar.right) {
          clippers.push(`${a.tagName}.${(a.className || '').toString().slice(0, 30)}`);
        }
      }
      a = a.parentElement;
    }
    return {
      clippers,
      inViewport:
        r.top >= 0 &&
        r.left >= 0 &&
        r.bottom <= document.documentElement.clientHeight &&
        r.right <= document.documentElement.clientWidth,
      hasSize: r.width > 0 && r.height > 0,
    };
  });

  expect(report.clippers).toEqual([]);
  expect(report.inViewport).toBe(true);
  expect(report.hasSize).toBe(true);
});

test('entity hover card dismisses on Escape', async ({ page, request }) => {
  await loginAs(page, { handle: 'TestPilot' });
  await setScenario(request, scenarioFor('entity_hover_escape', FIXTURES));
  await page.goto('/u/TestPilot');

  const link = page.locator(LINK);
  await expect(link).toBeVisible();
  await link.hover();
  const card = page.getByRole('tooltip', { name: /avenger stalker details/i });
  await expect(card).toBeVisible();

  await page.keyboard.press('Escape');
  await expect(card).toHaveCount(0);
});
