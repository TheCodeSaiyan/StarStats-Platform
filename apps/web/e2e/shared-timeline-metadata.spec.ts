/**
 * Phase 5 task 41: confirm the web's shared-profile pages never expose
 * a parser-submission UI.
 *
 * The tray's Review pane is the only surface that can submit unknown
 * lines — the design keeps parser submissions a single-source affordance
 * so the submission contract (`(shape_hash, client_anon_id)`) stays
 * one-to-one with installs. Any future shared per-event timeline must
 * NOT regress on this — hence the regression-only assertions below.
 *
 * Per-event metadata consumption (inferred badges, group-fold) is
 * landed as helpers under `src/lib/timeline-metadata.ts` and
 * `src/components/InferredBadge.tsx`, ready for the moment a per-event
 * shared-timeline endpoint exists. The current shared profile renders
 * day-bucket counts only; once a richer endpoint lands, swap the
 * heatmap for a per-event view that uses these helpers.
 */

import { expect, test } from '@playwright/test';
import {
  loginAs,
  notFound,
  publicSummaryShared,
  resetScenario,
  setScenario,
} from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

test('public profile renders without any parser-submission UI', async ({
  page,
  request,
}) => {
  await setScenario(request, {
    __id: 'shared_timeline_no_submission_ui_public',
    routes: {
      'GET /v1/public/JohnSomeone/summary': publicSummaryShared,
    },
  });

  await page.goto('/u/JohnSomeone');
  await expect(
    page.getByRole('heading', { name: 'JohnSomeone' }),
  ).toBeVisible();

  // No "Submit" / "Submit for review" / submission button anywhere on
  // the public profile. The Review pane lives in the tray app only.
  await expect(
    page.getByRole('button', { name: /submit/i }),
  ).toHaveCount(0);
});

test('shared profile renders without any parser-submission UI', async ({
  page,
  request,
}) => {
  await loginAs(page, { handle: 'TestPilot' });
  await setScenario(request, {
    __id: 'shared_timeline_no_submission_ui_shared',
    routes: {
      'GET /v1/public/JohnSomeone/summary': notFound,
      'GET /v1/u/JohnSomeone/summary': publicSummaryShared,
    },
  });

  await page.goto('/u/JohnSomeone');
  await expect(
    page.getByRole('heading', { name: 'JohnSomeone' }),
  ).toBeVisible();

  await expect(
    page.getByRole('button', { name: /submit/i }),
  ).toHaveCount(0);
});
