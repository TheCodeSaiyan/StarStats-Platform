/**
 * `/admin/chat/reports`: the moderator queue for chat reports. The revealed
 * messages are the only chat text StarStats ever holds, so the page must
 * show them with the warning that they cannot be proven, and the outcome
 * sent must be the one the moderator chose.
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

const REPORT_ID = '0199c000-0000-7000-8000-000000000001';

const report = {
  id: REPORT_ID,
  reporter_handle: 'SSDemoAlice',
  reported_handle: 'ssdemomallory',
  room_id: '!room:starstats.app',
  reason: 'harassment',
  details: 'kept going after I asked them to stop',
  messages: [
    {
      event_id: '$e1',
      sender: '@ssdemomallory:starstats.app',
      sent_at: '2026-09-28T12:00:00Z',
      text: 'something unkind',
    },
  ],
  status: 'open',
  created_at: '2026-09-28T12:05:00Z',
  resolved_at: null,
  resolved_by: null,
  resolution_note: null,
};

test('a moderator sees the revealed messages and restricts the player from chat', async ({
  page,
  request,
}) => {
  await resetScenario(request);
  await setScenario(
    request,
    scenarioFor('admin_chat_reports', {
      'GET /v1/auth/me': currentUser,
      'GET /v1/admin/chat/reports': { status: 200, body: { reports: [report] } },
      [`POST /v1/admin/chat/reports/${REPORT_ID}/resolve`]: {
        status: 200,
        body: { ...report, status: 'chat_restricted' },
      },
    }),
  );
  await loginAs(page, { handle: 'Mod', staffRoles: ['moderator'] });
  await page.goto('/admin/chat/reports', { timeout: 60_000 });

  const row = page.getByTestId('chat-report');
  await expect(row).toContainText('@ssdemomallory');
  await expect(row).toContainText('something unkind');
  await expect(row).toContainText('cannot be proven');

  page.once('dialog', (dialog) => {
    dialog.accept();
  });
  await row.getByRole('button', { name: 'Restrict from chat' }).click({ timeout: 30_000 });
  // The server action compiles on first use under `next dev`; give the
  // wait a server's budget, not a click's.
  await expect
    .poll(async () =>
      (await getCalls(request)).find(
        (c) => c.method === 'POST' && c.path === `/v1/admin/chat/reports/${REPORT_ID}/resolve`,
      )?.body,
      { timeout: 30_000 },
    )
    .toMatchObject({ outcome: 'chat_restricted' });
});
