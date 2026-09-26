/**
 * News: the admin console page that writes it, and /changelog, which shows
 * it to everyone.
 *
 * The rendering test asserts the property that differs between right and
 * wrong for untrusted text: a body containing markup must come out as the
 * literal characters, not as elements.
 */
import { test, expect } from '@playwright/test';
import {
  currentUser,
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

const POST = {
  id: '0199b000-0000-7000-8000-000000000001',
  title: 'Scheduled maintenance',
  body: 'Servers down 02:00-03:00 UTC.\n<b>not bold</b>',
  link_url: 'https://starstats.app/status',
  created_by: 'StarStatsDemo',
  created_at: '2026-09-26T10:00:00Z',
  updated_at: '2026-09-26T10:00:00Z',
  published_at: '2026-09-26T10:00:00Z',
};

const DRAFT = { ...POST, id: '0199b000-0000-7000-8000-000000000002', title: 'Draft idea', published_at: null };

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

test('the console lists posts and publishing lands on the published notice', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('news-admin', {
      'GET /v1/auth/me': currentUser,
      'GET /v1/admin/news': { status: 200, body: { posts: [POST, DRAFT] } },
      'POST /v1/admin/news/*': { status: 200, body: { ...DRAFT, published_at: '2026-09-26T11:00:00Z' } },
    }),
  );
  await loginAs(page, { handle: 'StarStatsDemo', staffRoles: ['admin'] });
  await page.goto('/admin/news', { timeout: 60_000 });

  const rows = page.getByTestId('news-admin-row');
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toContainText('Published');
  await expect(rows.nth(1)).toContainText('Draft');

  page.once('dialog', (d) => void d.accept());
  await rows.nth(1).getByRole('button', { name: 'Publish' }).click({ timeout: 30_000 });
  await expect(page).toHaveURL(/status=published/, { timeout: 30_000 });
  await expect(page.getByRole('status')).toContainText('Published.');
});

test('/changelog shows news as plain text, markup included literally', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('news-public', {
      'GET /v1/news': { status: 200, body: { posts: [POST] } },
      'GET /v1/roadmap/changelog': { status: 200, body: { entries: [] } },
    }),
  );
  await page.goto('/changelog', { timeout: 60_000 });
  const post = page.getByTestId('news-post');
  await expect(post).toHaveCount(1);
  await expect(post).toContainText('Scheduled maintenance');
  // The literal text is there, and no <b> element was created from it.
  await expect(post).toContainText('<b>not bold</b>');
  await expect(post.locator('b')).toHaveCount(0);
  await expect(post.getByRole('link', { name: 'Read more' })).toHaveAttribute(
    'href',
    'https://starstats.app/status',
  );
});
