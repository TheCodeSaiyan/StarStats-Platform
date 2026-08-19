import { expect, test } from '@playwright/test';
import {
  adminEventTypes,
  adminInferenceRulesListing,
  getCalls,
  loginAs,
  publishInferenceRuleResponse,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

/**
 * /admin/parser-inference-rules/{new,''} tests (Task 8).
 *
 * Covers the inference-rule authoring + management surfaces end-to-end:
 *   - The authoring form (`InferenceRuleForm`) lets a moderator pick a
 *     trigger event type, an emit event type, and add a followup
 *     pattern via structured `<select>`s and KV rows, then serialises
 *     that state into a single hidden `definition` field
 *     (`assembleRule`) that the server action re-parses and POSTs
 *     verbatim (no `enabled` flag — the server defaults new rules to
 *     enabled).
 *   - Submitting fires the native `window.confirm()` gate
 *     (ConfirmSubmitButton), then redirects to
 *     `/admin/parser-inference-rules?published=<id>` where a success
 *     chip renders.
 *   - The management list renders both an enabled and a retracted rule
 *     from the mocked `GET /v1/admin/parser-inference-rules` response,
 *     including the `trigger → emits` summary column.
 *   - Clicking "Retract" on an enabled row round-trips the row's full
 *     nested `definition` (trigger/followups/emits) as one hidden JSON
 *     blob and re-POSTs it with `enabled=false` — mirrors the #3
 *     parser-rules retract pattern but with a nested DTO instead of
 *     flat scalar fields.
 */

test('author_flow_submits_assembled_definition_and_redirects', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('inference_rules_author', {
      'GET /v1/admin/event-types': adminEventTypes,
      'GET /v1/admin/parser-inference-rules': adminInferenceRulesListing,
      'POST /v1/admin/parser-inference-rules': publishInferenceRuleResponse(
        'combat.kill_streak_v2',
        true,
      ),
    }),
  );
  await loginAs(page, { handle: 'Mod', staffRoles: ['moderator'] });

  await page.goto('/admin/parser-inference-rules/new');
  await expect(
    page.getByRole('heading', { name: 'Author inference rule' }),
  ).toBeVisible();

  await page.getByTestId('inference-rule-id-input').fill('combat.kill_streak_v2');
  await page.getByTestId('inference-rule-confidence-input').fill('0.82');
  await page.getByTestId('inference-rule-window-secs-input').fill('45');

  // Trigger: vehicle_destruction with one field_equals row.
  await page
    .getByTestId('inference-rule-trigger-event-type')
    .selectOption('vehicle_destruction');
  await page.getByTestId('inference-rule-trigger-field-equals-add').click();
  await page
    .getByTestId('inference-rule-trigger-field-equals-key-0')
    .fill('cause');
  await page
    .getByTestId('inference-rule-trigger-field-equals-value-0')
    .fill('combat');

  // Followup: add one row and pick its event type.
  await page.getByTestId('inference-rule-followup-add').click();
  await page
    .getByTestId('inference-rule-followup-0-event-type')
    .selectOption('player_death');

  // Emit: resolve_spawn with one templated field.
  await page
    .getByTestId('inference-rule-emit-event-type')
    .selectOption('resolve_spawn');
  await page.getByTestId('inference-rule-emit-fields-add').click();
  await page.getByTestId('inference-rule-emit-fields-key-0').fill('actor');
  await page
    .getByTestId('inference-rule-emit-fields-value-0')
    .fill('${trigger.actor}');

  page.once('dialog', (dialog) => {
    dialog.accept();
  });
  await page.getByTestId('inference-rule-submit').click();

  await expect(page).toHaveURL(
    /\/admin\/parser-inference-rules\?published=combat\.kill_streak_v2/,
  );
  await expect(page.getByRole('status')).toContainText(
    'Published combat.kill_streak_v2',
  );

  const calls = await getCalls(request);
  const publishCall = calls.find(
    (c) =>
      c.method === 'POST' && c.path === '/v1/admin/parser-inference-rules',
  );
  expect(publishCall?.body).toMatchObject({
    id: 'combat.kill_streak_v2',
    confidence: 0.82,
    window_secs: 45,
    trigger: {
      event_type: 'vehicle_destruction',
      field_equals: { cause: 'combat' },
    },
    followups: [{ event_type: 'player_death', field_equals: {} }],
    emits: {
      event_type: 'resolve_spawn',
      fields: { actor: '${trigger.actor}' },
    },
  });
});

// Rule row enabled=false is used to seed the "after retract" GET
// fixture — same rule_id + definition, flipped indicator, so the
// re-render is observable without inventing a new shape. Mirrors
// `rulesAfterRetract` in admin-parser-rules.spec.ts.
const rulesAfterRetract = {
  status: 200,
  body: {
    rules: [
      {
        ...adminInferenceRulesListing.body.rules[0],
        enabled: false,
      },
      adminInferenceRulesListing.body.rules[1],
    ],
  },
};

test('management_list_renders_both_rules_and_retract_posts_enabled_false', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('inference_rules_list', {
      'GET /v1/admin/parser-inference-rules': adminInferenceRulesListing,
    }),
  );
  await loginAs(page, { handle: 'Mod', staffRoles: ['moderator'] });

  await page.goto('/admin/parser-inference-rules');

  await expect(
    page.getByRole('heading', { name: 'Published inference rules' }),
  ).toBeVisible();

  const enabledRow = page
    .locator('tbody tr')
    .filter({ hasText: 'combat.kill_streak_v1' });
  await expect(enabledRow).toContainText('vehicle_destruction');
  await expect(enabledRow).toContainText('resolve_spawn');
  await expect(enabledRow).toContainText('0.75');
  await expect(enabledRow).toContainText('30');
  await expect(enabledRow).toContainText('✓');
  await expect(enabledRow.getByRole('button', { name: 'Retract' })).toBeVisible();

  const disabledRow = page
    .locator('tbody tr')
    .filter({ hasText: 'travel.jump_chain_v1' });
  await expect(disabledRow).toContainText('travel_jump');
  await expect(disabledRow).toContainText('0.6');
  await expect(disabledRow).toContainText('15');
  await expect(disabledRow).toContainText('—');
  await expect(disabledRow.getByRole('button', { name: 'Enable' })).toBeVisible();

  // Swap in the post-toggle GET fixture so the revalidate-then-render
  // pass shows the flipped state, mirroring admin-parser-rules.spec.ts.
  await setScenario(
    request,
    scenarioFor('inference_rules_after_retract', {
      'GET /v1/admin/parser-inference-rules': rulesAfterRetract,
      'POST /v1/admin/parser-inference-rules': publishInferenceRuleResponse(
        'combat.kill_streak_v1',
        false,
      ),
    }),
  );

  page.once('dialog', (dialog) => {
    dialog.accept();
  });
  await enabledRow.getByRole('button', { name: 'Retract' }).click();

  const rowAfter = page
    .locator('tbody tr')
    .filter({ hasText: 'combat.kill_streak_v1' });
  await expect(rowAfter).toContainText('—');
  await expect(rowAfter.getByRole('button', { name: 'Enable' })).toBeVisible();

  const calls = await getCalls(request);
  const toggleCall = calls.find(
    (c) =>
      c.method === 'POST' && c.path === '/v1/admin/parser-inference-rules',
  );
  expect(toggleCall?.body).toMatchObject({
    id: 'combat.kill_streak_v1',
    trigger: {
      event_type: 'vehicle_destruction',
      field_equals: { cause: 'combat' },
    },
    followups: [{ event_type: 'player_death', field_equals: {} }],
    emits: {
      event_type: 'resolve_spawn',
      fields: { actor: '${trigger.actor}' },
    },
    enabled: false,
  });
});
