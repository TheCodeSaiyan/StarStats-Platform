import { expect, test } from '@playwright/test';
import {
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

/**
 * Admin gating + sidebar visibility tests.
 *
 * The admin layout (`apps/web/src/app/admin/layout.tsx`) reads the
 * session cookie's `staffRoles` and:
 *   - redirects to `/auth/login?next=/admin` when not logged in
 *   - redirects directly to `/me` when logged in but lacking moderator/admin
 *   - lets the page render through otherwise
 *
 * The sidebar (`LeftRail.tsx`) hides the "Admin" link unless
 * `staffRoles.length > 0`.
 *
 * Tests below seed the cookie via `loginAs({ staffRoles })` rather than
 * walking the auth flow — every auth-flow test already covers session
 * minting; here we just verify gating behaves on the cookie shape.
 */

const emptyAdminQueue = {
  status: 200,
  body: { items: [], has_more: false },
};

const queueWithOneFlagged = {
  status: 200,
  body: {
    items: [
      {
        id: 'sub_flagged_001',
        submitter_handle: 'OtherPilot',
        pattern: 'foo bar',
        proposed_label: 'foo_event',
        description: 'Saw this in PU',
        sample_line: 'sample line text',
        log_source: 'live',
        status: 'flagged',
        rejection_reason: null,
        created_at: '2026-05-01T12:00:00Z',
        updated_at: '2026-05-01T12:00:00Z',
        vote_count: 0,
        flag_count: 3,
        viewer_voted: false,
        viewer_flagged: false,
      },
    ],
    has_more: false,
  },
};

test('non_staff_user_redirected_from_admin_to_me', async ({
  page,
  request,
}) => {
  await setScenario(request, scenarioFor('admin_gate_nonstaff'));
  await loginAs(page, { handle: 'TestPilot', staffRoles: [] });

  await page.goto('/admin');

  // Admin layout redirects non-staff directly to /me.
  await expect(page).toHaveURL(/\/me/);
});

test('non_staff_user_does_not_see_admin_link_in_sidebar', async ({
  page,
  request,
}) => {
  await setScenario(request, scenarioFor('admin_link_hidden'));
  await loginAs(page, { handle: 'TestPilot', staffRoles: [] });

  await page.goto('/dashboard');

  // The LeftRail conditionally renders the Admin link only when
  // staffRoles is non-empty — assert it's gone for a normal user.
  await expect(page.getByRole('link', { name: 'Admin' })).toHaveCount(0);
});

test('admin_user_sees_admin_link_and_landing_page_renders', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('admin_landing', {
      'GET /v1/admin/submissions/queue': emptyAdminQueue,
    }),
  );
  await loginAs(page, {
    handle: 'TheCodeSaiyan',
    staffRoles: ['admin'],
  });

  await page.goto('/admin');

  // No redirect — the layout let us through.
  await expect(page).toHaveURL(/\/admin\/?$/);

  // Landing renders the moderation dashboard heading — confirms the
  // admin layout let us through and the page actually rendered (not
  // just that the URL stuck).
  await expect(
    page.getByRole('heading', { name: 'Moderation' }),
  ).toBeVisible();

  // The Admin affordance now lives in the @handle account menu (Plan 4
  // nav restructure) rather than the left rail. Open the menu and
  // confirm the staff user sees the Admin item.
  await page.getByRole('button', { name: /@TheCodeSaiyan/ }).click();
  await expect(
    // Account menu is a disclosure (nav of links) since M-W10 — the
    // Admin affordance is a link, not a role=menuitem.
    page.getByRole('link', { name: 'Admin' }),
  ).toBeVisible();
});

test('moderator_user_can_access_admin_too', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('admin_landing_mod', {
      'GET /v1/admin/submissions/queue': queueWithOneFlagged,
    }),
  );
  await loginAs(page, {
    handle: 'ModCitizen',
    staffRoles: ['moderator'],
  });

  await page.goto('/admin');

  // Same gate — moderator passes.
  await expect(page).toHaveURL(/\/admin\/?$/);
});

/**
 * The three former config tabs (SMTP, Appearance, Ship Matrix) became
 * anchored sections of /admin/settings, and their old routes are
 * redirects. These are bookmarked operator routes, so the redirect is
 * behaviour worth pinning rather than an implementation detail.
 *
 * Asserting the landed heading as well as the URL: a redirect that
 * lands on a page which then throws would still satisfy toHaveURL.
 */
for (const [from, anchor] of [
  ['/admin/smtp', 'smtp'],
  ['/admin/appearance', 'appearance'],
  ['/admin/ship-matrix', 'ship-matrix'],
] as const) {
  test(`admin_${anchor.replace('-', '_')}_route_redirects_to_settings`, async ({
    page,
    request,
  }) => {
    await setScenario(request, scenarioFor(`admin_redirect_${anchor}`));
    await loginAs(page, {
      handle: 'TheCodeSaiyan',
      staffRoles: ['admin'],
    });

    await page.goto(from);

    await expect(page).toHaveURL(/\/admin\/settings/);
    await expect(
      page.getByRole('heading', { level: 1, name: 'Settings' }),
    ).toBeVisible();
    // The section it points at exists on the page it landed on.
    await expect(page.locator(`#${anchor}`)).toBeAttached();
  });
}

test('admin_settings_renders_all_three_config_sections', async ({
  page,
  request,
}) => {
  await setScenario(request, scenarioFor('admin_settings_sections'));
  await loginAs(page, { handle: 'TheCodeSaiyan', staffRoles: ['admin'] });

  await page.goto('/admin/settings');

  for (const heading of [
    'SMTP configuration',
    'Appearance defaults',
    'Ship Matrix enrichment',
  ]) {
    await expect(page.getByRole('heading', { name: heading })).toBeVisible();
  }
});
