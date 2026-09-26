/**
 * `/friends` — friends list, copy handle, notifications, the unread badge.
 *
 * Each assertion targets the property that differs between right and wrong,
 * not visibility: an unverified handle's Copy button is VISIBLE either way,
 * so the test reads `disabled` and the tooltip; the badge is asserted by its
 * number, not its presence.
 */
import { test, expect, type Page } from '@playwright/test';
import {
  currentUser,
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

const FIXTURES = {
  'GET /v1/auth/me': currentUser,
  'GET /v1/me/friends': {
    status: 200,
    body: {
      friends: [
        { handle: 'SSDemoWingman', since: '2026-08-01T12:00:00Z', rsi_verified: true },
        { handle: 'SSDemoUnproven', since: '2026-08-02T12:00:00Z', rsi_verified: false },
      ],
      incoming: [
        {
          id: '0199a000-0000-7000-8000-000000000001',
          requester_handle: 'SSDemoRecruit',
          recipient_handle: 'StarStatsDemo',
          status: 'pending',
          created_at: '2026-09-25T12:00:00Z',
          responded_at: null,
        },
      ],
      outgoing: [],
      friend_request_policy: 'everyone',
    },
  },
  'GET /v1/me/notifications': {
    status: 200,
    body: {
      unread_count: 3,
      items: [
        {
          id: '0199a000-0000-7000-8000-0000000000aa',
          kind: 'friend_request',
          actor_handle: 'SSDemoRecruit',
          payload: {
            request_id: '0199a000-0000-7000-8000-000000000001',
            rsi_verified: true,
          },
          created_at: '2026-09-25T12:00:00Z',
          read_at: null,
        },
      ],
    },
  },
  'POST /v1/me/friends/requests/*': {
    status: 200,
    body: {
      request: {
        id: '0199a000-0000-7000-8000-000000000001',
        requester_handle: 'SSDemoRecruit',
        recipient_handle: 'StarStatsDemo',
        status: 'accepted',
        created_at: '2026-09-25T12:00:00Z',
        responded_at: '2026-09-26T12:00:00Z',
      },
    },
  },
};

test.beforeEach(async ({ page, request }) => {
  await resetScenario(request);
  await setScenario(request, scenarioFor('friends', FIXTURES));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.setViewportSize({ width: 1440, height: 900 });
});

/**
 * The first visit in a run waits for `next dev` to compile the route, which
 * overruns the 10s navigation budget on a cold server. That budget is sized
 * for the page, not the compiler, so the wait gets a server-sized one.
 */
async function visit(page: Page): Promise<void> {
  await page.goto('/friends', { timeout: 60_000 });
}

async function openGroup(page: Page, name: string): Promise<void> {
  await page.locator('.hp-lens button', { hasText: name }).click();
}

test('copy handle is offered only for a verified RSI handle', async ({ page }) => {
  await visit(page);
  const verified = page
    .locator('.hp-grant', { hasText: '@SSDemoWingman' })
    .getByRole('button', { name: 'Copy handle' });
  const unverified = page
    .locator('.hp-grant', { hasText: '@SSDemoUnproven' })
    .getByRole('button', { name: 'Copy handle' });
  await expect(verified).toBeEnabled();
  await expect(unverified).toBeDisabled();
  await expect(unverified).toHaveAttribute('title', 'Handle not verified with RSI');
});

test('copy handle puts the exact handle on the clipboard', async ({ page, context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write']);
  await visit(page);
  await page
    .locator('.hp-grant', { hasText: '@SSDemoWingman' })
    .getByRole('button', { name: 'Copy handle' })
    .click({ timeout: 30_000 });
  await expect(
    page.locator('.hp-grant', { hasText: '@SSDemoWingman' }).getByRole('status').first(),
  ).toHaveText('Handle copied');
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe('SSDemoWingman');
});

test('the unread count reaches the account control', async ({ page }) => {
  await visit(page);
  await expect(page.locator('.hp-badge').first()).toHaveText(/3/);
});

test('accepting from the notifications inbox lands on the accepted notice', async ({
  page,
}) => {
  await visit(page);
  await openGroup(page, 'Notifications');
  const row = page.locator('.hp-grant', { hasText: 'sent you a friend request' });
  await expect(row.locator('.hp-chip', { hasText: 'new' })).toHaveCount(1);
  await row.getByRole('button', { name: 'Accept' }).click({ timeout: 30_000 });
  await expect(page).toHaveURL(/status=request_accepted/, { timeout: 30_000 });
  await expect(page.getByText('Request accepted — you are now friends.')).toBeVisible();
});

test('the page has exactly one h1, naming the page', async ({ page }) => {
  await visit(page);
  await expect(page.locator('h1')).toHaveCount(1);
  await expect(page.locator('h1')).toHaveText('Friends');
});
