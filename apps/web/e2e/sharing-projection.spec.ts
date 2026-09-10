/**
 * The sharing surface, in the projection.
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
        {
          recipient_handle: 'SSDemoQuartermaster',
          note: null,
          expires_at: '2026-08-01T12:00:00Z',
          view_count: 0,
          last_viewed_at: null,
          scope: { kind: 'full' },
        },
      ],
      org_shares: [{ org_slug: 'ssdemo-fleet' }],
    },
  },
  'GET /v1/me/shared-with-me': {
    status: 200,
    body: {
      shared_with_me: [
        {
          owner_handle: 'SSDemoNavigator',
          note: 'route data',
          expires_at: null,
        },
      ],
    },
  },
  'GET /v1/orgs': {
    status: 200,
    body: { orgs: [{ slug: 'ssdemo-fleet', name: 'SSDemo Fleet' }] },
  },
  'GET /v1/me/profile-views': {
    status: 200,
    body: {
      totals: {
        last_30d: 48,
        by_source_30d: { direct: 30, discover: 12, shared: 6 },
      },
      days: Array.from({ length: 30 }, (_, i) => ({
        day: `2026-08-${String(i + 1).padStart(2, '0')}`,
        total: [0, 1, 3, 5, 2, 0, 4][i % 7],
      })),
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
  await setScenario(request, scenarioFor('sharing-projection', FIXTURES));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.setViewportSize({ width: 1440, height: 900 });
});

async function openGroup(page: Page, name: string): Promise<void> {
  await page.locator('.hp-lens button', { hasText: name }).click();
}

test('the edit flow opens the outbound group and its editor', async ({
  page,
}) => {
  // The behaviour grouping put at risk: Edit navigates to
  // `?edit=<handle>#share-editor`, and `#share-editor` is a form INSIDE the
  // outbound section rather than a section of its own. Without the secondary
  // anchor the rail would stay on Visibility and the editor would not exist.
  // The real edit URL, as `buildEditHref` emits it: the handle plus the
  // share's current expiry and note, so the editor pre-fills rather than
  // silently clearing them on save.
  await page.goto(
    '/sharing?handle=SSDemoWingman&expires=2026-09-30T12%3A00%3A00.000Z&note=flight+lead#share-editor',
  );
  await expect(page.locator('.hp-settings')).toBeVisible();
  await expect(page.locator('#share-editor')).toBeVisible();
  await expect(page.getByLabel('RSI handle')).toHaveValue('SSDemoWingman');
  await expect(page.getByLabel('Note')).toHaveValue('flight lead');
});

test('the scope editor offers every scope tab', async ({ page }) => {
  // The port dropped these entirely at first: the "Specific tabs…" scope kind
  // was selectable with no tabs to select, so choosing it would have submitted
  // an empty set. The vocabulary mirrors ALLOWED_SCOPE_TABS in the Rust
  // validator, so a missing one is a scope the reader cannot grant.
  await page.goto('/sharing');
  await openGroup(page, 'Outbound');
  const boxes = page.locator('#share-editor input[name="scope_tabs"]');
  await expect(boxes).toHaveCount(6);
  for (const v of ['location', 'travel', 'combat', 'loadout', 'stability', 'commerce']) {
    await expect(
      page.locator(`#share-editor input[name="scope_tabs"][value="${v}"]`),
    ).toHaveCount(1);
  }
});

test('the page has exactly one h1, naming the page', async ({ page }) => {
  // Every flat screen these replaced had an h1; the projection has no titled
  // surface of its own, so it went missing on four ported pages at once
  // before a test noticed. The final crumb step carries it.
  await page.goto('/sharing');
  await expect(page.locator('h1')).toHaveCount(1);
  await expect(page.locator('h1')).toHaveText('Sharing');
});

test('no console errors across every group', async ({ page }) => {
  await page.goto('/sharing');
  await expect(page.locator('.hp-settings')).toBeVisible();
  for (const g of ['Outbound', 'Inbound', 'Views', 'Visibility']) {
    await openGroup(page, g);
    await page.waitForTimeout(250);
  }
  await page.waitForTimeout(900);
  if (consoleErrors.length) {
    console.log(`CONSOLE ERRORS:\n${consoleErrors.join('\n---\n')}`);
  }
  expect(consoleErrors).toEqual([]);
});

test('a SpiceDB outage says the authorisation service is offline, not "something went wrong"', async ({
  page,
  request,
}) => {
  // Regression for the 2026-09-09 production outage. SpiceDB had no
  // schema, so every RPC failed `FailedPrecondition` and the handlers
  // returned 500 `spicedb_error` — NOT the 503 this page used to match
  // on. All four load-bearing calls then landed in the all-failed
  // branch and the page rendered the generic "couldn't load your
  // sharing state" fallback, which tells the user nothing and reads
  // like a bug in their account rather than a service being down.
  //
  // The assertion that actually differs is WHICH banner shows: both
  // paths render a `bad`-toned BeamAlert and hide every section, so
  // asserting "an error is visible" passes on the broken code too.
  const outage = { status: 500, body: { error: 'spicedb_error' } };
  await setScenario(
    request,
    scenarioFor('sharing-projection', {
      ...FIXTURES,
      'GET /v1/me/visibility': outage,
      'GET /v1/me/shares': outage,
      'GET /v1/me/shared-with-me': outage,
      'GET /v1/orgs': outage,
    }),
  );

  await page.goto('/sharing');

  await expect(
    page.getByText('the authorisation service is offline'),
  ).toBeVisible();
  await expect(
    page.getByText("Couldn't load your sharing state"),
  ).toHaveCount(0);
});

test('the visibility toggle states what a stranger actually gets', async ({
  page,
  request,
}) => {
  // The old copy — "anyone can view your summary and timeline" — is true
  // of almost any setting, so it told the owner nothing, while the public
  // path had no clamp at all. Asserting "some copy is present" would have
  // passed on that. Assert the SPECIFIC clamp instead.
  await setScenario(
    request,
    scenarioFor('sharing-projection', {
      ...FIXTURES,
      'GET /v1/me/visibility': {
        status: 200,
        body: {
          public: true,
          listing_opt_out: false,
          public_scope: { kind: 'full', max_event_types: 5, window_days: 30 },
        },
      },
    }),
  );

  await page.goto('/sharing');

  await expect(
    page.getByText('Your 5 busiest event types, with counts'),
  ).toBeVisible();
  await expect(
    page.getByText('An activity heatmap covering the last 30 days'),
  ).toBeVisible();
});

test('an unclamped public profile is told it publishes everything', async ({
  page,
  request,
}) => {
  // A profile made public before the clamp existed still publishes its
  // full history. The owner cannot decide whether that is what they
  // wanted unless the page says so in those words, so this is the case
  // that most needs stating — and the one a cheerful default would hide.
  await setScenario(
    request,
    scenarioFor('sharing-projection', {
      ...FIXTURES,
      'GET /v1/me/visibility': {
        status: 200,
        body: { public: true, listing_opt_out: false },
      },
    }),
  );

  await page.goto('/sharing');

  await expect(
    page.getByText('Every event type you have logged, with counts'),
  ).toBeVisible();
  await expect(
    page.getByText(/Nothing is currently narrowing this profile/),
  ).toBeVisible();
});

test('editing a share keeps the scope it already has', async ({ page }) => {
  // The editor prefilled from URL params, which carry handle, note and
  // expiry but never the scope — so it rendered its own defaults and
  // saving rewrote a deliberately narrow share to the full manifest.
  // SSDemoWingman is fixtured with `scope: { kind: 'timeline' }`; before
  // the fix this select read "full", so a round-trip through the edit
  // form silently widened the grant.
  await page.goto(
    '/sharing?handle=SSDemoWingman&expires=2026-09-30T12%3A00%3A00.000Z&note=flight+lead#share-editor',
  );

  await expect(page.locator('#scope-kind')).toHaveValue('timeline');
});

test('a new share is time-boxed by default', async ({ page }) => {
  // The old default was the widest thing the form could express: full
  // manifest, no window, reached by typing a handle and pressing the
  // button. Narrow by default, widen deliberately.
  await page.goto('/sharing');
  await openGroup(page, 'Outbound');

  await expect(page.locator('#scope-window-days')).toHaveValue('30');
});

test('the public clamp is a control, not an announcement', async ({
  page,
  request,
}) => {
  // The clamp was enforced server-side from the moment public gained a
  // scope, and nothing in the UI could set it — the page listed the
  // settings and offered no way to choose them. Stating a decision made
  // on someone's behalf is a worse answer than showing nothing.
  //
  // Both assertions matter: the controls exist, AND they carry the stored
  // values rather than defaults, so the page describes what is actually
  // published.
  await setScenario(
    request,
    scenarioFor('public-scope-controls', {
      ...FIXTURES,
      'GET /v1/me/visibility': {
        status: 200,
        body: {
          public: true,
          listing_opt_out: false,
          public_scope: { kind: 'full', window_days: 30, max_event_types: 5 },
        },
      },
    }),
  );

  await page.goto('/sharing');

  await expect(page.locator('#public-max-types')).toHaveValue('5');
  await expect(page.locator('#public-window-days')).toHaveValue('30');
});

test('the clamp can be chosen before the profile is ever public', async ({
  page,
  request,
}) => {
  // A private profile still gets the controls. Choosing what going public
  // would publish BEFORE going public is the whole point — otherwise the
  // first thing that happens is a stranger can read something you have
  // not agreed to yet.
  await setScenario(
    request,
    scenarioFor('public-scope-while-private', {
      ...FIXTURES,
      'GET /v1/me/visibility': {
        status: 200,
        body: { public: false, listing_opt_out: false },
      },
    }),
  );

  await page.goto('/sharing');

  await expect(page.locator('#public-max-types')).toBeVisible();
  await expect(page.locator('#public-window-days')).toBeVisible();
  // No stored clamp yet, so the controls show the unclamped truth rather
  // than pre-selecting the default the server would seed.
  await expect(page.locator('#public-max-types')).toHaveValue('all');
});
