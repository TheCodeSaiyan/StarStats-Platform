import { expect, test } from '@playwright/test';
import {
  loginAs,
  orgDetail,
  ownedOrgs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

test.beforeEach(async ({ request, page }) => {
  await resetScenario(request);
  await loginAs(page);
});

test('orgs_index_renders_owned_orgs', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('orgs_index', {
      'GET /v1/orgs': ownedOrgs,
    }),
  );

  await page.goto('/orgs');

  await expect(page.getByText('Test Squadron')).toBeVisible();
  await expect(page.getByText('Aegis Pilots')).toBeVisible();
  await expect(
    page.getByRole('link', { name: 'Create org' }),
  ).toBeVisible();
});

test('create_new_org_redirects_to_detail', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('org_create', {
      'POST /v1/orgs': {
        status: 200,
        body: {
          org: {
            id: 'org_new',
            name: 'New Squadron',
            slug: 'new-squadron',
            owner_user_id: 'user_existing',
            created_at: '2026-05-04T00:00:00Z',
          },
        },
      },
      'GET /v1/orgs/new-squadron': orgDetail('new-squadron', 'New Squadron'),
    }),
  );

  await page.goto('/orgs/new');
  await page.getByLabel('Name').fill('New Squadron');
  await page.getByRole('button', { name: 'Create org' }).click();

  await expect(page).toHaveURL(/\/orgs\/new-squadron$/);
  await expect(
    page.getByRole('heading', { name: 'New Squadron' }),
  ).toBeVisible();
});

test('org_detail_shows_member_list', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('org_detail', {
      'GET /v1/orgs/test-squadron': orgDetail(
        'test-squadron',
        'Test Squadron',
      ),
    }),
  );

  await page.goto('/orgs/test-squadron');

  await expect(
    page.getByRole('heading', { name: 'Test Squadron' }),
  ).toBeVisible();
  // Member handles render as `<span className="mono">`. Use exact-text
  // match so the TopBar's `@TestPilot` (also a `span.mono`) doesn't trip
  // strict mode.
  await expect(page.getByText('TestPilot', { exact: true })).toBeVisible();
  await expect(page.getByText('WingmanOne', { exact: true })).toBeVisible();
});

test('add_member_form_submits', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('org_add_member_initial', {
      'GET /v1/orgs/test-squadron': orgDetail(
        'test-squadron',
        'Test Squadron',
      ),
    }),
  );

  await page.goto('/orgs/test-squadron');

  // Re-arm the scenario: the action issues a POST, then a redirect
  // back to the detail page that re-fetches GET /v1/orgs/:slug. The
  // re-fetch should include the new member so the assertion catches
  // a successful submit.
  await setScenario(request, {
    __id: 'org_add_member_after',
    routes: {
      'POST /v1/orgs/test-squadron/members': {
        status: 200,
        body: { added: true },
      },
      'GET /v1/orgs/test-squadron': {
        status: 200,
        body: {
          org: {
            id: 'org_test-squadron',
            name: 'Test Squadron',
            slug: 'test-squadron',
            owner_user_id: 'user_existing',
            created_at: '2026-01-01T00:00:00Z',
          },
          members: [
            { handle: 'TestPilot', role: 'owner' },
            { handle: 'WingmanOne', role: 'admin' },
            { handle: 'NewRecruit', role: 'member' },
          ],
          your_role: 'owner',
        },
      },
    },
  });

  await page.getByLabel('RSI handle').fill('NewRecruit');
  await page.getByRole('button', { name: 'Add to org' }).click();

  await expect(page).toHaveURL(/\/orgs\/test-squadron\?status=member_added/);
  await expect(page.locator('span.mono', { hasText: 'NewRecruit' })).toBeVisible();
});
