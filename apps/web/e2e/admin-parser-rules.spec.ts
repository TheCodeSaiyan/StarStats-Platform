import { expect, test } from '@playwright/test';
import {
  adminParserRulesListing,
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
 * /admin/parser-rules tests (Task 7).
 *
 * Covers the published-rules management surface:
 *   - The list renders both an enabled and a retracted rule from the
 *     mocked `GET /v1/admin/parser-rules` response.
 *   - Clicking "Retract" on an enabled row fires the native
 *     `window.confirm()` gate (ConfirmSubmitButton) and, once
 *     accepted, re-POSTs the row with `enabled=false` — the mock
 *     recorder is used to assert the exact body that went out.
 *   - After the server action revalidates, the page re-renders
 *     against the (swapped) post-toggle fixture, flipping the row's
 *     enabled indicator and button label.
 */

// Rule row enabled=false is used to seed the "after retract" GET
// fixture — same rule_id, flipped indicator, so the re-render is
// observable without inventing a new shape.
const rulesAfterRetract = {
  status: 200,
  body: {
    rules: [
      {
        ...adminParserRulesListing.body.rules[0],
        enabled: false,
      },
      adminParserRulesListing.body.rules[1],
    ],
  },
};

test('list_renders_both_seeded_rules', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('parser_rules_list', {
      'GET /v1/admin/parser-rules': adminParserRulesListing,
    }),
  );
  await loginAs(page, { handle: 'Mod', staffRoles: ['moderator'] });

  await page.goto('/admin/parser-rules');

  await expect(
    page.getByRole('heading', { name: 'Published rules' }),
  ).toBeVisible();

  const enabledRow = page.locator('tbody tr').filter({ hasText: 'combat.kill_v1' });
  await expect(enabledRow).toContainText('combat_kill');
  await expect(enabledRow).toContainText('✓');
  await expect(enabledRow.getByRole('button', { name: 'Retract' })).toBeVisible();

  const disabledRow = page.locator('tbody tr').filter({ hasText: 'travel.jump_v1' });
  await expect(disabledRow).toContainText('travel_jump');
  await expect(disabledRow).toContainText('—');
  await expect(disabledRow.getByRole('button', { name: 'Enable' })).toBeVisible();
});

test('retract_posts_enabled_false_and_rerenders', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('parser_rules_retract', {
      'GET /v1/admin/parser-rules': adminParserRulesListing,
      'POST /v1/admin/parser-rules': publishRuleResponse('combat.kill_v1', false),
    }),
  );
  await loginAs(page, { handle: 'Mod', staffRoles: ['moderator'] });

  await page.goto('/admin/parser-rules');
  const enabledRow = page.locator('tbody tr').filter({ hasText: 'combat.kill_v1' });
  await expect(enabledRow.getByRole('button', { name: 'Retract' })).toBeVisible();

  // Swap in the post-toggle GET fixture so the revalidate-then-render
  // pass shows the flipped state, mirroring the save-form pattern in
  // admin-parser-submissions.spec.ts.
  await setScenario(
    request,
    scenarioFor('parser_rules_after_retract', {
      'GET /v1/admin/parser-rules': rulesAfterRetract,
      'POST /v1/admin/parser-rules': publishRuleResponse('combat.kill_v1', false),
    }),
  );

  page.once('dialog', (dialog) => {
    dialog.accept();
  });
  await enabledRow.getByRole('button', { name: 'Retract' }).click();

  // Page re-renders: the same row now shows the retracted indicator
  // and an "Enable" button.
  const rowAfter = page.locator('tbody tr').filter({ hasText: 'combat.kill_v1' });
  await expect(rowAfter).toContainText('—');
  await expect(rowAfter.getByRole('button', { name: 'Enable' })).toBeVisible();

  const calls = await getCalls(request);
  const toggleCall = calls.find(
    (c) => c.method === 'POST' && c.path === '/v1/admin/parser-rules',
  );
  expect(toggleCall?.body).toMatchObject({
    rule_id: 'combat.kill_v1',
    event_name: 'combat_kill',
    match_kind: 'event_name',
    fields: ['actor', 'victim'],
    enabled: false,
  });
});
