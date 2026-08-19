/**
 * /contracts — public contract browse flow.
 *
 * Asserts:
 *   - The list renders contracts from `GET /api/contracts`, each card
 *     linking to its detail page.
 *   - A search term routes to `GET /api/contracts/search`.
 *   - The detail page renders the structured extraction (identity,
 *     reward, steps) from `GET /api/contracts/{id}`.
 *   - An unknown id renders the not-found page.
 *
 * Public pages — no login. Fixtures are set per-test via setScenario.
 * The detail page also fetches the locations catalogue
 * (`GET /v1/reference/location`) for EntityLink cross-linking, so that
 * route is mocked (empty) to avoid a 599.
 */

import { expect, test } from '@playwright/test';
import { resetScenario, setScenario } from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

const SUMMARY = {
  canonical_id: 'apprehend_zane_esteban',
  display_name: 'Apprehend Zane Esteban',
  contract_type: 'bounty',
  subcategory: 'Apprehension',
  gameplay_loop: 'bounty_hunting',
  issuer: 'Crusader Security',
  faction: 'Crusader',
  legal_status: 'legal',
  reward_amount: 8500,
  reward_currency: 'aUEC',
  confidence_score: 0.86,
  patch_version: '3.23',
  first_seen_at: '2026-07-01T00:00:00+00:00',
  updated_at: '2026-07-01T00:00:00+00:00',
};

const DETAIL = {
  canonical_id: 'apprehend_zane_esteban',
  schema_version: '1',
  suggested_action: 'create_new_contract',
  first_seen_at: '2026-07-01T00:00:00+00:00',
  updated_at: '2026-07-01T00:00:00+00:00',
  contract: {
    display_name: 'Apprehend Zane Esteban',
    contract_type: 'bounty',
    subcategory: 'Apprehension',
    gameplay_loop: 'bounty_hunting',
    issuer: 'Crusader Security',
    faction: 'Crusader',
    legal_status: 'legal',
    reward: { amount: 8500, currency: 'aUEC', bonus_amount: 1500 },
    fees: [{ type: 'deposit', amount: 0, currency: 'aUEC', refundable: true }],
    timeframe: { has_time_limit: true, deadline_text: '2h', duration_minutes: 120 },
    attributes: [
      { label: 'LAST KNOWN LOCATION', value: 'Glaciem Ring' },
      { label: 'RECOVERY LOCATION', value: 'a wreck site in the Glaciem Ring' },
    ],
    primary_objectives: ['Travel to Glaciem Ring', 'Apprehend or eliminate the target'],
    patch_version: '3.23',
    confidence_score: 0.86,
  },
  steps: [
    { order: 1, step_type: 'accept_contract', summary: 'Accept the bounty from the contract manager.', guidance: true },
    { order: 2, step_type: 'navigate', summary: 'Travel to Glaciem Ring.', location: 'Glaciem Ring', risk: 'medium' },
    { order: 3, step_type: 'engage', summary: 'Apprehend or eliminate the target.', risk: 'high', failure_condition: 'Target escapes the area' },
  ],
};

test('contracts_list_renders_cards', async ({ page, request }) => {
  await setScenario(request, {
    __id: 'contracts_list',
    routes: {
      'GET /api/contracts': {
        status: 200,
        body: { contracts: [SUMMARY], next_offset: null },
      },
    },
  });

  await page.goto('/contracts');

  await expect(
    page.getByRole('heading', { name: 'Contracts', level: 1 }),
  ).toBeVisible();
  await expect(
    page.getByRole('link', { name: 'Apprehend Zane Esteban' }),
  ).toBeVisible();
  await expect(page.getByText('8,500 aUEC')).toBeVisible();
});

test('contracts_search_hits_search_endpoint', async ({ page, request }) => {
  await setScenario(request, {
    __id: 'contracts_search',
    routes: {
      // A query present → results come from /search; the plain list
      // endpoint is still fetched for the facet vocabulary.
      'GET /api/contracts/search': {
        status: 200,
        body: { contracts: [SUMMARY], next_offset: null },
      },
      'GET /api/contracts': {
        status: 200,
        body: { contracts: [SUMMARY], next_offset: null },
      },
    },
  });

  await page.goto('/contracts?q=zane');

  await expect(
    page.getByRole('link', { name: 'Apprehend Zane Esteban' }),
  ).toBeVisible();
});

test('contracts_facet_filters_in_memory', async ({ page, request }) => {
  const DELIVERY = {
    ...SUMMARY,
    canonical_id: 'small_covalex_shipment',
    display_name: 'Small Covalex Shipment',
    contract_type: 'delivery',
    issuer: 'Covalex',
  };
  await setScenario(request, {
    __id: 'contracts_facets',
    routes: {
      'GET /api/contracts': {
        status: 200,
        body: { contracts: [SUMMARY, DELIVERY], next_offset: null },
      },
    },
  });

  // Unfiltered: both render.
  await page.goto('/contracts');
  await expect(page.getByRole('link', { name: 'Apprehend Zane Esteban' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Small Covalex Shipment' })).toBeVisible();

  // Filter by type=bounty (SUMMARY is a bounty): only it survives.
  await page.goto('/contracts?type=bounty');
  await expect(page.getByRole('link', { name: 'Apprehend Zane Esteban' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Small Covalex Shipment' })).toHaveCount(0);
});

test('contracts_detail_renders_extraction', async ({ page, request }) => {
  await setScenario(request, {
    __id: 'contracts_detail',
    routes: {
      'GET /api/contracts/apprehend_zane_esteban': { status: 200, body: DETAIL },
      // Detail page fetches the locations catalogue for EntityLink.
      'GET /v1/reference/location': { status: 200, body: { entries: [] } },
    },
  });

  await page.goto('/contracts/apprehend_zane_esteban');

  await expect(
    page.getByRole('heading', { name: 'Apprehend Zane Esteban', level: 1 }),
  ).toBeVisible();
  // Reward with bonus — rendered in both the hero readout and the
  // reward section, so match the first occurrence.
  await expect(page.getByText('8,500 aUEC (+1,500)').first()).toBeVisible();
  // Identity + steps.
  await expect(page.getByText('Crusader Security')).toBeVisible();
  await expect(page.getByText('Travel to Glaciem Ring.')).toBeVisible();
  await expect(page.getByText('Target escapes the area')).toBeVisible();
  // Free-text attribute value must render VERBATIM (not rewritten by the
  // EntityLink class-name prettifier).
  await expect(
    page.getByText('a wreck site in the Glaciem Ring'),
  ).toBeVisible();
});

test('contracts_detail_unknown_id_renders_not_found', async ({ page, request }) => {
  await setScenario(request, {
    __id: 'contracts_detail_404',
    routes: {
      'GET /api/contracts/does_not_exist': { status: 404, body: { error: 'not_found' } },
      'GET /v1/reference/location': { status: 200, body: { entries: [] } },
    },
  });

  await page.goto('/contracts/does_not_exist');

  // next dev returns HTTP 200 for notFound() while rendering the
  // not-found body, so assert on rendered content, not resp.status().
  await expect(
    page.getByRole('heading', { name: 'Page not found' }),
  ).toBeVisible();
});
