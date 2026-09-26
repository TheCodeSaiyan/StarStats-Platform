/**
 * /whats-new: news and shipped features for the signed-in player, with the
 * unread state the tray shares. Assertions read the property that differs
 * between right and wrong: the badge NUMBER, the per-item unread marker, and
 * that markup in a body arrives as text.
 */
import { test, expect } from '@playwright/test';
import {
  currentUser,
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

const NEWS = {
  items: [
    {
      id: '0199b000-0000-7000-8000-000000000001',
      title: 'Scheduled maintenance tonight',
      body: 'Sync pauses 02:00-03:00 UTC.\n<b>not bold</b>',
      link_url: null,
      published_at: '2026-09-26T10:00:00Z',
      unread: true,
    },
    {
      id: '0199b000-0000-7000-8000-000000000002',
      title: 'Older post',
      body: 'Already read.',
      link_url: null,
      published_at: '2026-09-20T10:00:00Z',
      unread: false,
    },
  ],
  unread_count: 1,
};

const ITEMS = {
  seen_via_auth: true,
  items: [
    {
      roadmap_item_id: '0199c000-0000-7000-8000-000000000001',
      slug: 'social-friends',
      title: 'Friends',
      headline_status: 'shipped',
      latest_changelog_entry_id: '0199c000-0000-7000-8000-000000000002',
      latest_published_at: '2026-09-26T09:00:00Z',
      unread: true,
    },
  ],
};

test.beforeEach(async ({ page, request }) => {
  await resetScenario(request);
  await setScenario(
    request,
    scenarioFor('whats-new', {
      'GET /v1/auth/me': currentUser,
      'GET /v1/me/news': { status: 200, body: NEWS },
      'GET /v1/me/roadmap/whats-new': { status: 200, body: ITEMS },
      'POST /v1/me/news/*': { status: 204 },
      'POST /v1/me/roadmap/whats-new/seen': { status: 204 },
    }),
  );
  await loginAs(page, { handle: 'StarStatsDemo' });
  await page.setViewportSize({ width: 1440, height: 900 });
});

test('the account badge counts unread news and features together', async ({ page }) => {
  await page.goto('/whats-new', { timeout: 60_000 });
  // 1 unread post + 1 unread feature.
  await expect(page.locator('.hp-badge').first()).toHaveText(/2/);
});

test('only unread entries carry the new marker, and bodies are text', async ({ page }) => {
  await page.goto('/whats-new', { timeout: 60_000 });
  const posts = page.getByTestId('whatsnew-news');
  await expect(posts).toHaveCount(2);
  await expect(posts.nth(0)).toHaveAttribute('data-unread', 'true');
  await expect(posts.nth(1)).not.toHaveAttribute('data-unread', 'true');
  await expect(posts.nth(0)).toContainText('<b>not bold</b>');
  await expect(posts.nth(0).locator('b')).toHaveCount(0);
});

test('mark all read clears both feeds', async ({ page }) => {
  await page.goto('/whats-new', { timeout: 60_000 });
  await page.getByRole('button', { name: 'Mark all read' }).click({ timeout: 30_000 });
  await expect(page).toHaveURL(/status=all_read/, { timeout: 30_000 });
  await expect(page.getByText('All caught up.')).toBeVisible();
});
