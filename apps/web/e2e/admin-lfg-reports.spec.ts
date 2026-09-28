/**
 * `/admin/lfg/reports`: resolving a report sends the outcome the moderator
 * clicked. It did not: the queue's buttons carry the outcome as their
 * name/value, which a form action's FormData leaves out, so every resolve
 * reloaded the queue having done nothing. See ConfirmSubmitButton.
 */
import { test, expect } from '@playwright/test';
import {
  currentUser,
  getCalls,
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

const REPORT_ID = '0199c000-0000-7000-8000-000000000002';

const report = {
  id: REPORT_ID,
  post_id: '0199b000-0000-7000-8000-000000000009',
  reporter_handle: 'SSDemoAlice',
  host_handle: 'SSDemoSpammer',
  reason: 'spam',
  details: null,
  post_snapshot: { activity: 'mining', note: 'buy credits here' },
  status: 'open',
  created_at: '2026-09-28T12:05:00Z',
  resolved_at: null,
  resolved_by: null,
  resolution_note: null,
};

for (const [button, outcome, confirms] of [
  ['Dismiss', 'dismissed', false],
  ['Remove post', 'post_removed', true],
] as const) {
  test(`${button} sends outcome ${outcome}`, async ({ page, request }) => {
    await resetScenario(request);
    await setScenario(
      request,
      scenarioFor('admin_lfg_reports', {
        'GET /v1/auth/me': currentUser,
        'GET /v1/admin/lfg/reports': { status: 200, body: { reports: [report] } },
        [`POST /v1/admin/lfg/reports/${REPORT_ID}/resolve`]: {
          status: 200,
          body: { ...report, status: outcome },
        },
      }),
    );
    await loginAs(page, { handle: 'Mod', staffRoles: ['moderator'] });
    await page.goto('/admin/lfg/reports', { timeout: 60_000 });
    if (confirms) {
      page.once('dialog', (dialog) => {
        dialog.accept();
      });
    }
    await page
      .getByTestId('lfg-report')
      .getByRole('button', { name: button, exact: true })
      .click({ timeout: 30_000 });
    // The server action compiles on first use under `next dev`.
    await expect
      .poll(
        async () =>
          (await getCalls(request)).find(
            (c) => c.method === 'POST' && c.path === `/v1/admin/lfg/reports/${REPORT_ID}/resolve`,
          )?.body,
        { timeout: 30_000 },
      )
      .toMatchObject({ outcome });
  });
}
