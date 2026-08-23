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
