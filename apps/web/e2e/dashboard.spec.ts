/**
 * /dashboard redirect spec.
 *
 * As of Mirror Plan 4, /dashboard is a redirect stub that forwards all
 * traffic to /me (the unified private home). The original page content
 * (Top types, heatmap, event stream) is superseded by /me's widget
 * canvas. These tests verify only the redirect behaviour.
 *
 * NOTE: The original dashboard_renders_top_types_and_timeline,
 * dashboard_clicking_event_type_drills_down, and
 * dashboard_pager_older_link_uses_smallest_seq tests have been removed
 * because they asserted content that no longer exists at /dashboard.
 * Equivalent coverage for the /me page belongs in a me.spec.ts.
 */

import { expect, test } from '@playwright/test';
import {
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

test.beforeEach(async ({ request, page }) => {
  await resetScenario(request);
  await loginAs(page);
});

test('dashboard_redirects_to_me', async ({ page, request }) => {
  await setScenario(request, scenarioFor('dashboard_redirect_check'));

  await page.goto('/dashboard');

  // /dashboard is now a redirect stub — the browser should land on /me.
  await expect(page).toHaveURL(/\/me/);
});
