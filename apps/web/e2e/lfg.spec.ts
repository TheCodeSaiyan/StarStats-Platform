/**
 * Looking for Group: the board, the host's panel and the post form.
 */
import { test, expect, type Page } from '@playwright/test';
import { currentUser, loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

const MY_POST = '0199b000-0000-7000-8000-000000000001';
const OTHER_POST = '0199b000-0000-7000-8000-000000000002';

const post = (id: string, host: string, extra: Record<string, unknown> = {}) => ({
  id,
  host_handle: host,
  activity: 'mining',
  system: 'Stanton',
  location: 'Everus Harbor',
  ship: 'Prospector',
  crew_slots: 2,
  voice: 'optional',
  region: 'eu',
  note: 'Quantanium run',
  created_at: '2026-09-27T12:00:00Z',
  expires_at: '2099-01-01T00:00:00Z',
  closed_at: null,
  removed_at: null,
  crew_count: 0,
  host_verified: true,
  my_status: null,
  is_host: false,
  ...extra,
});

const FIXTURES = {
  'GET /v1/auth/me': currentUser,
  'GET /v1/lfg/options': {
    status: 200,
    body: {
      activities: ['mining', 'salvage', 'fps'],
      systems: ['Nyx', 'Pyro', 'Stanton'],
      voices: ['none', 'optional', 'required'],
      regions: ['any', 'eu', 'na'],
      crew_min: 1,
      crew_max: 30,
      expiry_default_minutes: 120,
      expiry_min_minutes: 15,
      expiry_max_minutes: 360,
    },
  },
  'GET /v1/lfg': {
    status: 200,
    body: {
      posts: [
        post(OTHER_POST, 'SSDemoMiner', { activity: 'salvage', crew_count: 1 }),
        post(MY_POST, 'StarStatsDemo', { is_host: true }),
      ],
    },
  },
  [`GET /v1/lfg/${MY_POST}`]: {
    status: 200,
    body: {
      ...post(MY_POST, 'StarStatsDemo', { is_host: true }),
      members: [
        {
          handle: 'SSDemoRecruit',
          status: 'requested',
          created_at: '2026-09-27T12:05:00Z',
          responded_at: null,
        },
      ],
    },
  },
  'GET /v1/me/crew': {
    status: 200,
    body: {
      windows: [
        {
          post_id: OTHER_POST,
          activity: 'salvage',
          ended_at: '2026-09-27T14:00:00Z',
          closes_at: '2099-01-01T00:00:00Z',
          crew: [
            { handle: 'SSDemoMiner', my_commend: 'good_comms' },
            { handle: 'SSDemoRecruit', my_commend: null },
          ],
        },
      ],
      history: [
        {
          post_id: OTHER_POST,
          handle: 'SSDemoMiner',
          activity: 'salvage',
          flew_at: '2026-09-27T12:10:00Z',
        },
      ],
    },
  },
};

test.beforeEach(async ({ page, request }) => {
  await resetScenario(request);
  await setScenario(request, scenarioFor('lfg', FIXTURES));
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.setViewportSize({ width: 1440, height: 900 });
});

async function visit(page: Page, query = ''): Promise<void> {
  await page.goto(`/lfg${query}`, { timeout: 60_000 });
}

async function openGroup(page: Page, name: string): Promise<void> {
  await page.locator('.hp-lens button', { hasText: name }).click();
}

test('the board lists open posts with what a player needs to decide', async ({ page }) => {
  await visit(page);
  const posts = page.getByTestId('lfg-post');
  await expect(posts).toHaveCount(2);
  const other = posts.filter({ hasText: 'SSDemoMiner' });
  await expect(other).toContainText('Salvage with @SSDemoMiner');
  await expect(other).toContainText('1/2 crew');
  await expect(other).toContainText('Voice optional');
  await expect(other.getByRole('button', { name: 'Ask to join' })).toBeVisible();
  // Your own post is marked, with nothing to join and nothing to report.
  const mine = posts.filter({ hasText: '@StarStatsDemo' });
  await expect(mine.getByText('Your post')).toBeVisible();
  await expect(mine.getByText('Report this post')).toHaveCount(0);
});

test('accepting a player reports what the server recorded', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('lfg_accept', {
      ...FIXTURES,
      // The server declined instead, as it would if the group had filled.
      [`PUT /v1/lfg/${MY_POST}/members/SSDemoRecruit`]: {
        status: 200,
        body: {
          handle: 'SSDemoRecruit',
          status: 'declined',
          created_at: '2026-09-27T12:05:00Z',
          responded_at: '2026-09-27T12:06:00Z',
        },
      },
    }),
  );
  await visit(page);
  await openGroup(page, 'Your group');
  const asking = page.getByTestId('lfg-asking');
  await expect(asking).toContainText('@SSDemoRecruit');
  await asking.getByRole('button', { name: 'Accept' }).click({ timeout: 30_000 });
  await expect(page).toHaveURL(/status=member_declined/, { timeout: 30_000 });
});

test('posting without a verified handle says why', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('lfg_post_unverified', {
      ...FIXTURES,
      'GET /v1/lfg': { status: 200, body: { posts: [] } },
      'POST /v1/lfg': { status: 403, body: { error: 'rsi_handle_not_verified' } },
    }),
  );
  await visit(page);
  await openGroup(page, 'Post');
  await page
    .locator('form:has(#lfg-activity)')
    .getByRole('button', { name: 'Post', exact: true })
    .click({ timeout: 30_000 });
  await expect(page).toHaveURL(/error=rsi_handle_not_verified/, { timeout: 30_000 });
  await expect(page.getByText('Verify your RSI handle first')).toBeVisible();
});

test('the crew you flew with can be commended, one word each', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('lfg_commend', {
      ...FIXTURES,
      [`PUT /v1/crew/${OTHER_POST}/commends/SSDemoRecruit`]: {
        status: 200,
        body: { post_id: OTHER_POST, recipient: 'SSDemoRecruit', kind: 'reliable' },
      },
    }),
  );
  await visit(page);
  await openGroup(page, 'Crew');
  const mates = page.getByTestId('commend-mate');
  await expect(mates).toHaveCount(2);
  // What you already gave is shown, and pressed.
  const miner = mates.filter({ hasText: '@SSDemoMiner' });
  await expect(miner).toContainText('You said: Good comms');
  await expect(miner.getByRole('button', { name: 'Good comms' })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await expect(page.getByTestId('crew-history')).toContainText('@SSDemoMiner');
  const recruit = mates.filter({ hasText: '@SSDemoRecruit' });
  await expect(recruit).toContainText('Not commended yet');
  await recruit.getByRole('button', { name: 'Reliable' }).click({ timeout: 30_000 });
  await expect(page).toHaveURL(/status=commended/, { timeout: 30_000 });
  await expect(page.getByText('They are told the word, not who gave it')).toBeVisible();
});
