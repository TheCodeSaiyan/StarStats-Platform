/**
 * `/sharing` layout regressions.
 *
 * Every assertion here is the property that ACTUALLY DIFFERS between the
 * broken and fixed render, per the repo rule: a `toBeVisible()` on any of
 * these passes on the broken code too. The summary was visible at 104px tall,
 * the crumb heading was visible while drawn on top of the tab rail, and a
 * 453px-wide three-option select is as visible as a 300px one.
 *
 * Measured values at the time of the fix are quoted in each case so a future
 * reader can tell a regression from a deliberate retune.
 */
import { test, expect } from '@playwright/test';
import {
  currentUser,
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

const FIXTURES = {
  'GET /v1/auth/me': currentUser,
  'GET /v1/me/visibility': {
    status: 200,
    body: { public: true, listing_opt_out: false },
  },
  'GET /v1/me/shares': {
    status: 200,
    body: {
      shares: [
        {
          recipient_handle: 'SSDemoWingman',
          note: 'flight lead',
          expires_at: '2026-09-30T12:00:00Z',
          view_count: 4,
          last_viewed_at: '2026-08-20T09:00:00Z',
          scope: { kind: 'timeline' },
        },
      ],
      org_shares: [],
    },
  },
  'GET /v1/me/shared-with-me': { status: 200, body: { shared_with_me: [] } },
  'GET /v1/orgs': { status: 200, body: { orgs: [] } },
  'GET /v1/me/profile-views': {
    status: 200,
    body: { totals: { last_30d: 0, by_source_30d: {} }, days: [] },
  },
};

test.beforeEach(async ({ page, request }) => {
  await resetScenario(request);
  await setScenario(request, scenarioFor('sharing-layout', FIXTURES));
  await loginAs(page, { handle: 'StarStatsDemo' });
});

test('the more-options disclosure reads as one line, not a log-row grid', async ({
  page,
}) => {
  // It carried `.hp-lg` — the event-log row — which is
  // `grid-template-columns: 64px 1fr 92px`. The label landed in the 64px
  // column and wrapped one word per line: measured 104px tall for a
  // single-line label. A single line of `--fs-micro` is under 30px.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/sharing');
  await page.locator('.hp-lens button', { hasText: 'Outbound' }).click();
  const summary = page.locator('details.hp-disclosure > summary');
  await expect(summary).toBeVisible();
  const box = await summary.boundingBox();
  expect(box).not.toBeNull();
  expect(box!.height).toBeLessThan(40);
});

test('the crumb heading does not overlap the lens tabs on a phone', async ({
  page,
}) => {
  // The legacy mobile clamp (`h1:not(.hud-tile__title) { font-size: 26px
  // !important }`, ≤640px) beat `.hp-crumb h1 { font-size: inherit }` and grew
  // the heading's absolutely-positioned box from ~13px to 34px tall, pushing it
  // from top:42 down to bottom:76 — through the tab rail pinned at top:62.
  // Measured overlaps at 390px: 61x14px with "Inbound", 30x14px with
  // "Outbound". Both elements stay `visible` throughout, so only the geometry
  // tells the two renders apart.
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/sharing');
  const heading = page.locator('.hp-crumb h1');
  await expect(heading).toBeVisible();
  const h = (await heading.boundingBox())!;
  const tabs = page.locator('.hp-lens button');
  const count = await tabs.count();
  expect(count).toBeGreaterThan(0);
  for (let i = 0; i < count; i += 1) {
    const t = (await tabs.nth(i).boundingBox())!;
    const overlaps =
      h.x < t.x + t.width &&
      t.x < h.x + h.width &&
      h.y < t.y + t.height &&
      t.y < h.y + h.height;
    expect(
      overlaps,
      `crumb heading overlaps tab ${i} (${JSON.stringify(h)} vs ${JSON.stringify(t)})`,
    ).toBe(false);
  }
});

test('pane titles keep the projection size on a phone', async ({ page }) => {
  // Same clamp, second victim: `h2:not(.hud-tile__title) { font-size: 22px
  // !important }` beat the projection's own `.hp-phd h2 { font-size: 17px }`
  // (patterns-holo.css, ≤700px), so every pane title rendered 5px over its
  // designed size and "Shared with specific handles" wrapped to two lines
  // against its context line.
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/sharing');
  const title = page.locator('.hp-phd h2').first();
  await expect(title).toBeVisible();
  await expect(title).toHaveCSS('font-size', '17px');
});

test('form controls do not stretch to the width of the pane', async ({
  page,
}) => {
  // `.hp-formrow > .hp-field { flex: 1 }` in a 1040px pane gave a four-option
  // select 453px and the read-only `/u/<handle>` field 1002px. Nothing else on
  // the page is that wide, which is what made the surface read as unfinished.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/sharing');
  const widths = await page.evaluate(() =>
    Array.from(document.querySelectorAll('.hp-formrow .hp-input')).map((el) => ({
      name: el.getAttribute('name') ?? el.id,
      w: Math.round(el.getBoundingClientRect().width),
    })),
  );
  expect(widths.length).toBeGreaterThan(0);
  for (const c of widths) {
    expect(c.w, `${c.name} is ${c.w}px wide`).toBeLessThanOrEqual(360);
  }
});

test('the document sits in a reading column, not the full-width pane', async ({
  page,
}) => {
  // Prose is capped at 62ch (467px measured) while the pane ran to 1040px, so
  // every paragraph stopped at 45% of its own panel. The two have to agree on
  // one measure; this surface takes the narrower one.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/sharing');
  const inner = page.locator('.hp-settings__inner');
  await expect(inner).toBeVisible();
  const box = (await inner.boundingBox())!;
  expect(box.width).toBeLessThanOrEqual(860);
});
