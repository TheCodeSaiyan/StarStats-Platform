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
  // Two cold routes in one case — `/orgs/new` and then `/orgs/[slug]` — and the
  // config's navigation budget is sized for a warm one. Under full-suite load
  // the dev server compiles both on demand.
  test.slow();
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
  // Wait for the surface before touching the form. The page is a projection
  // now, so the tree above this form is client-rendered — and a submit that
  // lands before React has attached is swallowed rather than posted. It failed
  // only in a full-suite run and never in isolation, which is the signature of
  // that race and not of flake; the same shape has bitten this suite twice.
  await expect(page.locator('.hp-stage')).toBeVisible();
  // Wait for the FORM, not just the surface. The projection shell renders
  // before the page body streams in, so `.hp-stage` being visible does not mean
  // the button exists yet — a retry loop around the click then spends its whole
  // budget waiting for an element that has not arrived.
  const submit = page.getByRole('button', { name: 'Create org' });
  await expect(submit).toBeVisible({ timeout: 15_000 });
  await page.getByLabel('Name').fill('New Squadron');

  await expect(async () => {
    // GUARD THE RETRY. The click is retried because a submit landing before
    // hydration is swallowed — but the retry is not idempotent: once the first
    // click HAS navigated (just slowly, `/orgs/[slug]` compiling cold), the
    // next pass looks for "Create org" on the DETAIL page, waits out its own
    // 5s locator timeout and burns the budget on the wrong screen. That is how
    // this failed in a full-suite run, and the stored page snapshot showed the
    // detail page already rendered underneath the timeout.
    if (new URL(page.url()).pathname === '/orgs/new') {
      await submit.click();
    }
    await expect(page).toHaveURL(/\/orgs\/new-squadron$/, { timeout: 5000 });
  }).toPass({ timeout: 30_000 });
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
