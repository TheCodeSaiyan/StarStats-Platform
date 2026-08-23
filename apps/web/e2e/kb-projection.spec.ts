/**
 * The kb surface, in the projection.
 *
 * NOT a capture spec any more. This file began as scaffolding for the port —
 * a set of `goto` + `waitForTimeout` + `screenshot` cases whose only job was
 * producing images to judge, plus the fixtures they needed. Those 28 cases
 * asserted nothing, slept for half a second each, and are gone; what is left
 * are the assertions written alongside them, which are about behaviour and
 * outlive the port.
 */
import { test, expect } from '@playwright/test';
import {
  kbDetail,
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';


test.beforeEach(async ({ request }) => {
  await resetScenario(request);
  await setScenario(request, scenarioFor('kb-projection'));
});

test('the flat marketing chrome does not stack above the volume', async ({
  page,
}) => {
  // `/kb` is public. The layout's signed-out branch used to render `MarketingNav`
  // and the marketing footer. It sets `display: flex` INLINE, so hiding it
  // needs `!important` — a plain rule loses to the inline declaration however
  // specific it is. Shipped stacked once, on the first public projection page.
  await page.goto('/kb/vehicle');
  await expect(page.locator('.hp-catgrid')).toBeVisible();
  await expect(page.locator('.ss-marketing-nav')).toBeHidden();
  await expect(page.locator('.site-footer')).toBeHidden();
});

test('a signed-out visitor gets Sign in, not an account menu', async ({
  page,
}) => {
  // `/kb` is public. The chrome must not offer the reader's own areas, and
  // must not claim "Projection live" when no uplink is streaming.
  await page.goto('/kb');
  await expect(page.locator('.hp-settings')).toBeVisible();
  await expect(page.locator('.hp-signin')).toBeVisible();
  await expect(page.locator('.hp-acct')).toHaveCount(0);
  await expect(page.locator('.hp-top .live')).toHaveCount(0);
});

test('a signed-out visitor is not shown the labels of pages they cannot open', async ({
  page,
}) => {
  // Hiding a link is presentation, not protection — but offering one is a
  // promise, and "Sharing"/"Calibrate" tell an outsider what exists and invite
  // a bounce off a login wall.
  // Chrome collapse is MEASURED, not a breakpoint, and the measurement lands a
  // frame or two after mount — so `if (await toggle.isVisible()) click()` races
  // it: the toggle can read visible on the check and be hidden by the time the
  // click lands. (It did, order-dependently, once the file grew.) Pin a
  // viewport wide enough that the nav is inline and assert the toggle is gone
  // first, which both removes the race and states the expectation.
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto('/kb');
  await expect(page.locator('.hp-settings')).toBeVisible();
  await expect(page.locator('.hp-navtoggle')).toBeHidden();
  const nav = page.locator('.hp-lk');
  await expect(nav.getByText('Docs', { exact: true })).toBeVisible();
  await expect(nav.getByText('Sharing', { exact: true })).toHaveCount(0);
  await expect(nav.getByText('Calibrate', { exact: true })).toHaveCount(0);
});

test('search, facet and sort survive as URL state', async ({ page }) => {
  // The browse is a URL: shareable, bookmarkable, back-button correct, and it
  // works with JavaScript off. The hidden fields are what stop a search from
  // silently resetting the reader's sort.
  await page.goto('/kb/vehicle?sort=manufacturer&dir=desc');
  await expect(page.locator('.hp-catgrid')).toBeVisible();
  await page.getByLabel('Search').fill('aegis');
  await page.getByRole('button', { name: 'Search' }).click();
  await expect(page).toHaveURL(/q=aegis/);
  await expect(page).toHaveURL(/sort=manufacturer/);
  await expect(page).toHaveURL(/dir=desc/);
});

test('a card link is nameable — the article landmark would otherwise hide it', async ({
  page,
}) => {
  // `aria-label` on the card link is load-bearing: the contents sit inside
  // `<article>`, an ARIA landmark, and the accessible-name algorithm stops at
  // landmark boundaries.
  await page.goto('/kb/vehicle');
  const first = page.locator('.hp-catcard').first();
  await expect(first).toHaveAttribute('aria-label', /.+/);
});

test('the page has exactly one h1, naming the page', async ({ page }) => {
  await page.goto('/kb');
  await expect(page.locator('h1')).toHaveCount(1);
  await expect(page.locator('h1')).toHaveText('Knowledge base');
});

test('signed in, the chrome offers the account menu', async ({
  page,
  request,
}) => {
  await setScenario(request, scenarioFor('kb-capture-authed'));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.goto('/kb');
  await expect(page.locator('.hp-acct')).toBeVisible();
  await expect(page.locator('.hp-signin')).toHaveCount(0);
});

/**
 * The entity DETAIL sheet.
 *
 * These assert what a green typecheck, a green build and a green unit suite
 * all pass without: that the sheet is inside the projection at all, that it
 * has exactly one h1 and that h1 names the entry, that the empty case leaves
 * no titled pane wrapped around nothing, and that a heading a nested component
 * owns is not announced twice.
 *
 * The entry fixture goes through `kbDetail`, not a hand-written literal — the
 * mock server reads `body`, and a hand-rolled `json` key would have produced a
 * silently empty response.
 */
const SLUG_ROUTE = 'GET /v1/reference/vehicle/slug/aegis-avenger-stalker';

function detailScenario(metadata: Record<string, unknown> = { speed: { scm: 210 } }) {
  const base = scenarioFor('kb-projection');
  return {
    ...base,
    __id: 'kb-capture-detail',
    routes: {
      ...base.routes,
      [SLUG_ROUTE]: kbDetail({
        category: 'vehicle',
        class_name: 'AEGS_Avenger_Stalker',
        display_name: 'Aegis Avenger Stalker',
        slug: 'aegis-avenger-stalker',
        summary: {
          manufacturer: 'Aegis Dynamics',
          role: 'Fighter',
          hull_size: 'Small',
        },
        metadata,
      }),
    },
  };
}

const DETAIL_URL = '/kb/vehicle/aegis-avenger-stalker';

test('the detail sheet renders in the projection, not the flat shell', async ({
  page,
  request,
}) => {
  await setScenario(request, detailScenario());
  await page.goto(DETAIL_URL);
  // The volume and the flat chrome it replaces are mutually exclusive.
  //
  // These tests run SIGNED OUT. `MarketingNav` used to be the flat chrome here
  // and has since been DELETED — so asserting it is hidden would now pass on an
  // element that does not exist, which is the trap this suite has hit twice.
  // `.site-footer` still exists in `layout.tsx` and is still hidden by
  // `projection-shell.css`, so that is the assertion with something behind it.
  await expect(page.locator('.hp-stage')).toBeVisible();
  await expect(page.locator('.site-footer')).toHaveCount(0);
});

test('the detail sheet has exactly one h1, naming the entry', async ({
  page,
  request,
}) => {
  await setScenario(request, detailScenario());
  await page.goto(DETAIL_URL);
  await expect(page.locator('h1')).toHaveCount(1);
  await expect(page.locator('h1')).toHaveText('Aegis Avenger Stalker');
});

test('an unreferenced entry gets no empty Contracts pane', async ({
  page,
  request,
}) => {
  await setScenario(request, detailScenario());
  await page.goto(DETAIL_URL);
  await expect(page.locator('.hp-settings')).toBeVisible();
  // Assert on the PANE HEADER specifically. A substring match anywhere in the
  // sheet would pass on the word "contract" appearing in unrelated copy — the
  // failure mode that let two earlier guards go green over live bugs.
  await expect(
    page.locator('.hp-phd h2', { hasText: /^Contracts$/ }),
  ).toHaveCount(0);
});

test('the Ship Matrix heading is announced once, not twice', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    detailScenario({
      speed: { scm: 210 },
      ship_matrix: {
        name: 'Aegis Avenger Stalker',
        description: 'A bounty hunter variant.',
        manufacturer: { name: 'Aegis Dynamics', code: 'AEGS' },
      },
    }),
  );
  await page.goto(DETAIL_URL);
  await expect(page.getByRole('heading', { name: 'Ship Matrix' })).toHaveCount(
    1,
  );
});

test('a referenced entry gets a Contracts pane, headed once', async ({
  page,
  request,
}) => {
  // The negative case above only proves the gate closes. This proves it opens
  // — and that suppressing the component's own `<h2>` did not leave the
  // section unheaded, which is the way that fix could have gone wrong.
  const base = detailScenario();
  await setScenario(request, {
    ...base,
    __id: 'kb-capture-detail-contracts',
    routes: {
      ...base.routes,
      'GET /api/contracts/by-entity': {
        status: 200,
        body: {
          contracts: [
            {
              canonical_id: 'apprehend_zane_esteban',
              display_name: 'Apprehend Zane Esteban',
              contract_type: 'bounty',
              subcategory: 'Apprehension',
              gameplay_loop: 'bounty_hunting',
              issuer: 'Crusader Security',
              faction: 'Crusader',
              legal_status: 'legal',
              reward_amount: 8500,
              reward_currency: 'aUEC',
              confidence_score: 0.86,
              patch_version: '3.23',
              first_seen_at: '2026-07-01T00:00:00+00:00',
              updated_at: '2026-07-01T00:00:00+00:00',
            },
          ],
          next_offset: null,
        },
      },
    },
  });
  await page.goto(DETAIL_URL);
  await expect(page.locator('.hp-phd h2', { hasText: /^Contracts$/ })).toHaveCount(1);
  await expect(
    page.getByRole('link', { name: 'Apprehend Zane Esteban' }),
  ).toBeVisible();
});

/**
 * The browse screen against the SPEC, not against itself.
 *
 * `Catalogue.jsx` is one of the three screens COVERAGE marks as read from real
 * source, and the first pass at `/kb/[category]` never opened it — the cards,
 * the chips and the pager were invented, and the overlay comparison was left
 * out altogether. These assert the shapes the spec actually calls for.
 */
test('the browse cards are planes, not a bespoke card', async ({ page }) => {
  await page.goto('/kb/vehicle');
  const cards = page.locator('.hp-catgrid .hp-catcard');
  await expect(cards.first()).toBeVisible();
  // `Catalogue.jsx` draws each entry as `hp-plane flat`.
  await expect(cards.first().locator('.hp-plane.flat')).toHaveCount(1);
  // …and the invented classes are gone. Spelled out rather than interpolated
  // so a future rename sweep cannot quietly turn this into an assertion that
  // the NEW classes are absent — which is what happened the first time.
  await expect(page.locator('[class*="hp-kbcard"]')).toHaveCount(0);
  await expect(page.locator('[class*="hp-kbgrid"]')).toHaveCount(0);
});

test('facets and sort share one chip style', async ({ page }) => {
  // The spec has a single `chip(active)` used by both rows. Two near-identical
  // control styles on one screen is exactly what a design system exists to
  // prevent.
  await page.goto('/kb/vehicle');
  const chips = page.locator('.hp-catchip');
  expect(await chips.count()).toBeGreaterThan(2);
  await expect(page.locator('.hp-preset, .hp-sortrow')).toHaveCount(0);
});

test('the browse offers the overlay comparison', async ({ page }) => {
  // Missing entirely from the first pass. It sits between the sort row and the
  // grid, and nothing is fetched until two entries are picked.
  await page.goto('/kb/vehicle');
  const bar = page.locator('.hp-cmp-bar');
  await expect(bar).toBeVisible();
  await expect(page.getByText(/Pick two or three to overlay them/)).toBeVisible();
});

test('exactly one skip link', async ({ page }) => {
  // Deleting the flat chrome left the root layout's skip link pointing at a
  // wrapper that no longer contained anything to skip, alongside the
  // projection's own — two "Skip to content" links on every page.
  await page.goto('/kb/vehicle');
  await expect(page.getByRole('link', { name: /skip to content/i })).toHaveCount(
    1,
  );
});

test('the browse makes no API call until a comparison is asked for', async ({
  page,
}) => {
  // The browse catalogue is BUILD-TIME static since the M10 cutover — zero
  // runtime API calls. The comparison overlay is the only thing on the page
  // that talks to the API, and it goes through a per-IP rate-limited endpoint
  // that has already caused a 429 wave once when render-time fetching crept in.
  //
  // Asserted by watching the network, because the guard is about WHEN the
  // request happens and no assertion about the rendered page can see that.
  const compareCalls: string[] = [];
  page.on('request', (r) => {
    if (r.url().includes('/kb/compare/')) compareCalls.push(r.url());
  });

  await page.goto('/kb/vehicle');
  await expect(page.locator('.hp-cmp-bar')).toBeVisible();
  await page.waitForTimeout(800);
  expect(compareCalls).toEqual([]);

  // One pick is still not a comparison.
  await page.locator('.hp-cmp-bar .picks button').first().click();
  await page.waitForTimeout(500);
  expect(compareCalls).toEqual([]);

  // Two is.
  await page.locator('.hp-cmp-bar .picks button').nth(1).click();
  await expect.poll(() => compareCalls.length).toBeGreaterThan(0);
});
