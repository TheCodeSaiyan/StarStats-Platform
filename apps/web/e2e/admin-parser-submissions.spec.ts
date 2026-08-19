import { expect, test } from '@playwright/test';
import {
  getCalls,
  loginAs,
  publishRuleResponse,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

/**
 * /admin/parser-submissions tests (W6).
 *
 * Covers the rule-author moderation surface end-to-end:
 *   - Non-staff users are bounced by /admin/layout.tsx.
 *   - The list renders mocked rows in the popularity order the
 *     server returned (we trust the server-side sort — the test
 *     just verifies the page wires it through).
 *   - Status-filter pills change the visible bucket via the URL.
 *   - Clicking a row navigates to the detail page and renders the
 *     full payload (raw examples, shell tag, partial structured).
 *   - Submitting the moderation form fires PATCH and the page
 *     re-renders against the updated mock fixture.
 */

const pendingListing = {
  status: 200,
  body: {
    submissions: [
      {
        id: 101,
        shape_hash: 'sh_aaa111',
        first_submitted_at: '2026-05-01T12:00:00Z',
        last_submitted_at: '2026-05-10T09:00:00Z',
        submitter_count: 7,
        total_occurrence_count: 42,
        status: 'pending',
        shell_tag: 'ItemPort',
        raw_example_preview: '<X> picked up <Y>',
      },
      {
        id: 102,
        shape_hash: 'sh_bbb222',
        first_submitted_at: '2026-05-02T12:00:00Z',
        last_submitted_at: '2026-05-09T09:00:00Z',
        submitter_count: 3,
        total_occurrence_count: 5,
        status: 'pending',
        shell_tag: null,
        raw_example_preview: '<X> died',
      },
    ],
    next_after: null,
  },
};

const dismissedListing = {
  status: 200,
  body: {
    submissions: [
      {
        id: 201,
        shape_hash: 'sh_dis999',
        first_submitted_at: '2026-04-01T12:00:00Z',
        last_submitted_at: '2026-04-05T09:00:00Z',
        submitter_count: 1,
        total_occurrence_count: 1,
        status: 'dismissed',
        shell_tag: null,
        raw_example_preview: 'spam line',
      },
    ],
    next_after: null,
  },
};

const detail101 = {
  status: 200,
  body: {
    id: 101,
    shape_hash: 'sh_aaa111',
    first_submitted_at: '2026-05-01T12:00:00Z',
    last_submitted_at: '2026-05-10T09:00:00Z',
    submitter_count: 7,
    total_occurrence_count: 42,
    status: 'pending',
    reviewer_notes: null,
    rule_id: null,
    payload: {
      shape_hash: 'sh_aaa111',
      raw_examples: [
        "<2026-05-10T09:00:00.000Z> [Notice] <ItemPort> grabbed <Hand>",
        "<2026-05-10T09:01:00.000Z> [Notice] <ItemPort> grabbed <Helmet>",
      ],
      partial_structured: { actor: 'PlayerName' },
      shell_tag: 'ItemPort',
      suggested_event_name: null,
      suggested_field_names: null,
      notes: null,
      context_examples: [],
      game_build: 'EA-3.24.0-LIVE',
      channel: 'live',
      occurrence_count: 7,
      client_anon_id: 'anon_test',
    },
  },
};

// Post-PATCH detail — same id, status flipped, notes + rule_id set.
const detail101AfterSave = {
  status: 200,
  body: {
    ...detail101.body,
    status: 'rule_written',
    reviewer_notes: 'shipped as combat.kill',
    rule_id: 'rule_test_42',
  },
};

test('non_staff_user_redirected_from_parser_submissions_to_me', async ({
  page,
  request,
}) => {
  await setScenario(request, scenarioFor('parser_admin_gate'));
  await loginAs(page, { handle: 'TestPilot', staffRoles: [] });

  await page.goto('/admin/parser-submissions');

  // Admin layout enforces the role gate and redirects directly to /me (Mirror Plan 4).
  await expect(page).toHaveURL(/\/me/);
});

test('list_renders_mocked_submissions_in_server_order', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('parser_admin_list', {
      'GET /v1/admin/parser-submissions': pendingListing,
    }),
  );
  await loginAs(page, {
    handle: 'TheCodeSaiyan',
    staffRoles: ['admin'],
  });

  await page.goto('/admin/parser-submissions');

  await expect(
    page.getByRole('heading', { name: 'Parser shapes' }),
  ).toBeVisible();

  const table = page.getByTestId('parser-submissions-table');
  await expect(table).toBeVisible();

  // Server returned aaa111 first, bbb222 second — the page must
  // preserve that order.
  const rows = table.locator('tbody tr');
  await expect(rows).toHaveCount(2);
  await expect(rows.nth(0)).toContainText('sh_aaa111');
  await expect(rows.nth(0)).toContainText('ItemPort');
  await expect(rows.nth(0)).toContainText('7');
  await expect(rows.nth(0)).toContainText('42');
  await expect(rows.nth(1)).toContainText('sh_bbb222');
});

test('status_filter_changes_visible_bucket', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('parser_admin_filter', {
      'GET /v1/admin/parser-submissions': pendingListing,
    }),
  );
  await loginAs(page, { handle: 'Mod', staffRoles: ['moderator'] });

  await page.goto('/admin/parser-submissions');
  await expect(page.getByTestId('parser-submissions-table')).toContainText(
    'sh_aaa111',
  );

  // Swap the fixture so the dismissed bucket returns its row, then
  // click the filter pill.
  await setScenario(
    request,
    scenarioFor('parser_admin_dismissed', {
      'GET /v1/admin/parser-submissions': dismissedListing,
    }),
  );

  await page.getByRole('link', { name: 'Dismissed' }).click();
  await expect(page).toHaveURL(/status=dismissed/);
  await expect(page.getByTestId('parser-submissions-table')).toContainText(
    'sh_dis999',
  );
  await expect(
    page.getByTestId('parser-submissions-table'),
  ).not.toContainText('sh_aaa111');
});

test('row_link_opens_detail_page_with_payload', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('parser_admin_detail_nav', {
      'GET /v1/admin/parser-submissions': pendingListing,
      'GET /v1/admin/parser-submissions/101': detail101,
    }),
  );
  await loginAs(page, { handle: 'Admin', staffRoles: ['admin'] });

  await page.goto('/admin/parser-submissions');
  await page.getByRole('link', { name: /sh_aaa111/ }).first().click();

  await expect(page).toHaveURL(/\/admin\/parser-submissions\/101/);

  // Header — the shape hash renders as h1.
  await expect(
    page.getByRole('heading', { name: 'sh_aaa111' }),
  ).toBeVisible();

  // Raw examples — both lines render as <pre> blocks.
  const rawExamples = page.getByTestId('raw-example');
  await expect(rawExamples).toHaveCount(2);
  await expect(rawExamples.first()).toContainText('grabbed <Hand>');
  await expect(rawExamples.nth(1)).toContainText('grabbed <Helmet>');

  // Metadata table picks up shell_tag and channel.
  await expect(page.getByText('shell_tag').first()).toBeVisible();
  await expect(page.getByText('ItemPort').first()).toBeVisible();
});

test('save_form_patches_status_notes_and_rule_id', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('parser_admin_save', {
      'GET /v1/admin/parser-submissions': pendingListing,
      'GET /v1/admin/parser-submissions/101': detail101,
      'PATCH /v1/admin/parser-submissions/101': {
        status: 200,
        body: detail101AfterSave.body,
      },
    }),
  );
  await loginAs(page, { handle: 'Admin', staffRoles: ['admin'] });

  await page.goto('/admin/parser-submissions/101');
  await expect(
    page.getByRole('heading', { name: 'sh_aaa111' }),
  ).toBeVisible();

  // Swap in the post-save GET fixture so the revalidate-then-render
  // pass shows the new state.
  await setScenario(
    request,
    scenarioFor('parser_admin_after_save', {
      'GET /v1/admin/parser-submissions': pendingListing,
      'GET /v1/admin/parser-submissions/101': detail101AfterSave,
      'PATCH /v1/admin/parser-submissions/101': {
        status: 200,
        body: detail101AfterSave.body,
      },
    }),
  );

  await page
    .getByTestId('parser-submission-status-select')
    .selectOption('rule_written');
  await page
    .getByTestId('parser-submission-notes-input')
    .fill('shipped as combat.kill');
  await page
    .getByTestId('parser-submission-rule-id-input')
    .fill('rule_test_42');

  await page.getByTestId('parser-submission-save').click();

  // After the action returns + revalidate fires, the page re-renders
  // against the updated fixture. We assert the new status surfaces in
  // the header strip.
  await expect(page.getByText('Current status: rule_written')).toBeVisible();
});

// Post-publish detail — same id, rule_id + status set from the
// "Publish rule" panel's server action (publishRuleAction).
const detail101AfterPublish = {
  status: 200,
  body: {
    ...detail101.body,
    status: 'rule_written',
    rule_id: 'combat.kill_v3',
  },
};

test('publish_rule_form_posts_rule_patches_submission_and_redirects', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('parser_admin_publish', {
      'GET /v1/admin/parser-submissions': pendingListing,
      'GET /v1/admin/parser-submissions/101': detail101,
      'POST /v1/admin/parser-rules': publishRuleResponse('combat.kill_v3', true),
      'PATCH /v1/admin/parser-submissions/101': {
        status: 200,
        body: detail101AfterPublish.body,
      },
    }),
  );
  await loginAs(page, { handle: 'Admin', staffRoles: ['admin'] });

  await page.goto('/admin/parser-submissions/101');
  await expect(
    page.getByRole('heading', { name: 'sh_aaa111' }),
  ).toBeVisible();

  // Swap in the post-publish GET fixture so the redirect target
  // (a fresh navigation, not just a revalidate) renders the linked +
  // advanced submission state.
  await setScenario(
    request,
    scenarioFor('parser_admin_after_publish', {
      'GET /v1/admin/parser-submissions': pendingListing,
      'GET /v1/admin/parser-submissions/101': detail101AfterPublish,
      'POST /v1/admin/parser-rules': publishRuleResponse('combat.kill_v3', true),
      'PATCH /v1/admin/parser-submissions/101': {
        status: 200,
        body: detail101AfterPublish.body,
      },
    }),
  );

  await page
    .getByTestId('parser-submission-publish-rule-id-input')
    .fill('combat.kill_v3');
  await page
    .getByTestId('parser-submission-publish-event-name-input')
    .fill('combat_kill');
  await page
    .getByTestId('parser-submission-publish-match-kind-select')
    .selectOption('event_name');
  await page
    .getByTestId('parser-submission-publish-body-regex-input')
    .fill('(?P<who>\\w+)');
  await page
    .getByTestId('parser-submission-publish-fields-input')
    .fill('who');

  page.once('dialog', (dialog) => {
    dialog.accept();
  });
  await page.getByTestId('parser-submission-publish-submit').click();

  // Redirect target carries the rule id the mock's POST response
  // returned (not necessarily the submitted value — the page trusts
  // the API response, per the component's comment).
  await expect(page).toHaveURL(/\?published=combat\.kill_v3/);
  await expect(page.getByRole('status')).toContainText(
    'Published rule combat.kill_v3',
  );

  const calls = await getCalls(request);
  const publishCall = calls.find(
    (c) => c.method === 'POST' && c.path === '/v1/admin/parser-rules',
  );
  expect(publishCall?.body).toMatchObject({
    rule_id: 'combat.kill_v3',
    event_name: 'combat_kill',
    match_kind: 'event_name',
    body_regex: '(?P<who>\\w+)',
    fields: ['who'],
    enabled: true,
  });

  const patchCall = calls.find(
    (c) =>
      c.method === 'PATCH' && c.path === '/v1/admin/parser-submissions/101',
  );
  expect(patchCall?.body).toMatchObject({
    rule_id: 'combat.kill_v3',
    status: 'rule_written',
  });
});

// ---------------------------------------------------------------------------
// Task 8 — "Publish to community" panel (community promotion).
//
// INSPECT-ONLY in this environment: Playwright needs a browser + the mock
// server, neither of which runs here. Written to mirror the neighbouring
// `publish_rule_form_...` test; run with `pnpm --filter web e2e` locally.
// ---------------------------------------------------------------------------

// Detail with the shape already promoted — community_submission_id set.
// Renders the linked "Published to community" badge instead of the form.
const detail101AfterCommunity = {
  status: 200,
  body: {
    ...detail101.body,
    community_submission_id: 'comm-uuid-1',
  },
};

test('publish_community_form_promotes_shape_and_links_badge', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('parser_admin_community_publish', {
      'GET /v1/admin/parser-submissions': pendingListing,
      'GET /v1/admin/parser-submissions/101': detail101,
      'POST /v1/admin/parser-submissions/101/publish': {
        status: 201,
        body: {
          community_submission_id: 'comm-uuid-1',
          already_published: false,
        },
      },
    }),
  );
  await loginAs(page, { handle: 'Admin', staffRoles: ['admin'] });

  await page.goto('/admin/parser-submissions/101');
  await expect(
    page.getByRole('heading', { name: 'sh_aaa111' }),
  ).toBeVisible();

  // Prefill: `pattern` ← payload.raw_examples[0].
  await expect(
    page.getByTestId('parser-submission-community-pattern-input'),
  ).toHaveValue(/grabbed <Hand>/);

  // Swap in the post-publish GET fixture so the redirect target (a fresh
  // navigation) renders the linked badge instead of the form.
  await setScenario(
    request,
    scenarioFor('parser_admin_after_community_publish', {
      'GET /v1/admin/parser-submissions': pendingListing,
      'GET /v1/admin/parser-submissions/101': detail101AfterCommunity,
      'POST /v1/admin/parser-submissions/101/publish': {
        status: 201,
        body: {
          community_submission_id: 'comm-uuid-1',
          already_published: false,
        },
      },
    }),
  );

  await page
    .getByTestId('parser-submission-community-label-input')
    .fill('combat.grab');
  await page
    .getByTestId('parser-submission-community-description-input')
    .fill('Item grabbed from a port.');
  // Owner decision: moderator force-anonymize override.
  await page
    .getByTestId('parser-submission-community-force-anonymous-input')
    .check();

  page.once('dialog', (dialog) => {
    dialog.accept();
  });
  await page.getByTestId('parser-submission-community-submit').click();

  // Success chip is derived from the API response's
  // community_submission_id, never the submitted form.
  await expect(page).toHaveURL(/\?community=comm-uuid-1/);
  await expect(
    page
      .getByRole('status')
      .filter({ hasText: 'Published to community' })
      .first(),
  ).toBeVisible();

  // The promoted shape now renders the linked badge pointing at the
  // public community entry (/submissions/<id>) instead of the form.
  await expect(
    page.getByTestId('parser-submission-community-view-link'),
  ).toHaveAttribute('href', '/submissions/comm-uuid-1');

  // POST body carries label/description/pattern + the force_anonymous
  // override the moderator ticked.
  const calls = await getCalls(request);
  const publishCall = calls.find(
    (c) =>
      c.method === 'POST' &&
      c.path === '/v1/admin/parser-submissions/101/publish',
  );
  expect(publishCall?.body).toMatchObject({
    proposed_label: 'combat.grab',
    description: 'Item grabbed from a port.',
    pattern: expect.stringContaining('grabbed <Hand>'),
    force_anonymous: true,
  });
});
