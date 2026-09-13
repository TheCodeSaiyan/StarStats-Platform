import { expect, test } from '@playwright/test';
import { loginAs, scenarioFor, setScenario } from './helpers/api-mock';

/**
 * The InfoTip popover opened correctly for a long time and was still
 * invisible: as an absolutely-positioned child it was clipped by every
 * scrolling ancestor of a widget tile. Measured on /me at v1.8.170 — the
 * popover rendered at y=[177,290] while its own tile spanned [266,344].
 *
 * A visibility assertion does NOT catch that (the element is `visible`;
 * it is the ancestors that cut it off), and neither does a unit test in
 * jsdom, which has no layout. This spec asserts the property that
 * actually matters: no ancestor of the popover clips it, and it lands
 * inside the viewport.
 *
 * Hosted on `/u/[handle]`. It was moved here when `/me` became the projection
 * and lost its widget tiles; the public profile has now followed, so the
 * affordance under test is `BeamTip` — the projection's own derivation tip —
 * rather than the flat `InfoTip`. The DEFECT is identical and so are the
 * assertions: `.hp-plane` and the pane clip exactly as `.hud-tile` and
 * `.hud-tile__body` did, and `BeamTip` portals out for that reason (its own
 * comment cites this bug).
 *
 * The copy is still the registry's (`INFERENCE_EXPLANATIONS`), so a tip that
 * empties still fails here.
 */
const TRAVEL_FIXTURES = {
      // `/u/[handle]` as the OWNER: page.tsx short-circuits to the self
      // path before hitting the public endpoints, so these are
      // belt-and-suspenders stubs that keep the scenario deterministic.
      'GET /v1/public/TestPilot/summary': { status: 404, body: {} },
      'GET /v1/public/TestPilot/rsi-profile': { status: 404, body: {} },
      'GET /v1/public/TestPilot/rsi-orgs': { status: 200, body: { orgs: [] } },
  'GET /v1/users/me/profile-layout': {
    status: 200,
    body: { layout: [{ id: 'travel', enabled: true, size: 'compact' }] },
  },
  'GET /v1/me/metrics/event-types': {
    status: 200,
    body: {
      types: [
        { event_type: 'quantum_target_selected', count: 42 },
        { event_type: 'join_pu', count: 7 },
        { event_type: 'change_server', count: 3 },
      ],
    },
  },
  'GET /v1/me/stats/routes': { status: 200, body: { routes: [] } },
  'GET /v1/me/stats/travel': {
    status: 200,
    body: { quantum_jumps: 42, planets_visited: ['Hurston'] },
  },
};

test('infotip popover is not clipped by its widget tile', async ({ page, request }) => {
  await loginAs(page, { handle: 'TestPilot' });
  await setScenario(request, scenarioFor('infotip_unclipped', TRAVEL_FIXTURES));
  await page.goto('/u/TestPilot');

  // `BeamTip` opens on CLICK — the projection's tip is a lit hairline button,
  // not a hover affordance like the flat `InfoTip` this replaced. The defect
  // under test is unchanged: where the popover lands once it is open.
  const btn = page.locator('button.hp-tip__t').first();
  await expect(btn).toBeVisible();
  await btn.click();

  const pop = page.locator('.hp-tip__pop').first();
  await expect(pop).toBeVisible();

  const report = await page.evaluate(() => {
    const el = document.querySelector('.hp-tip__pop') as HTMLElement;
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

// `fleet` attaches its tip to a RankedList `note`, not a labelled readout —
// the note was a joined string and had to become JSX to carry the element.
// That is a different attachment shape from every other widget, so it gets
// its own assertion rather than riding on travel's.
test('infotip renders when attached to a list note (fleet)', async ({ page, request }) => {
  await loginAs(page, { handle: 'TestPilot' });
  await setScenario(
    request,
    scenarioFor('infotip_fleet_note', {
      'GET /v1/users/me/profile-layout': {
        status: 200,
        body: { layout: [{ id: 'fleet', enabled: true, size: 'compact' }] },
      },
      'GET /v1/me/stats/fleet': {
        status: 200,
        body: {
          ships: [
            { vehicle_class: 'RSI_Aurora_MR', trip_count: 12 },
            { vehicle_class: 'AEGS_Avenger_Titan', trip_count: 5 },
          ],
        },
      },
    }),
  );
  await page.goto('/u/TestPilot');

  const btn = page.locator('.hp-note button.hp-tip__t').first();
  await expect(btn).toBeVisible();
  await btn.click();

  const pop = page.locator('.hp-tip__pop').first();
  await expect(pop).toBeVisible();
  // The honest bit: flown, not owned.
  await expect(pop).toContainText(/not ships you own/i);
});

test('infotip explanation is readable and dismissable', async ({ page, request }) => {
  await loginAs(page, { handle: 'TestPilot' });
  await setScenario(request, scenarioFor('infotip_readable', TRAVEL_FIXTURES));
  await page.goto('/u/TestPilot');

  const btn = page.locator('button.hp-tip__t').first();
  await expect(btn).toBeVisible();
  await btn.click();

  // The travel widget's first tip explains quantum jumps; assert on the
  // copy so a registry edit that empties the text fails here.
  const pop = page.locator('.hp-tip__pop').first();
  await expect(pop).toContainText(/quantum/i);

  await page.keyboard.press('Escape');
  await expect(page.locator('.hp-tip__pop')).toHaveCount(0);
});
