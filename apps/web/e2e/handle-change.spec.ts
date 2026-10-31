/**
 * Changing the RSI handle from the settings "RSI handle ownership" pane.
 *
 * What these guard: nothing changes until the user has seen what the change
 * does (the confirm step), the session moves to the new handle from the
 * SERVER's answer, a refusal says why and changes nothing, and a verified
 * user can re-prove their handle without losing verification.
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
import { liveIn } from './helpers/shell';

const VERIFIED_ME = {
  status: 200,
  body: { ...currentUser.body, rsi_verified: true, totp_enabled: false },
};

const STATUS_VERIFIED = {
  status: 200,
  body: {
    claimed_handle: 'TestPilot',
    verified: true,
    free_change_available: false,
    cooldown_days: 30,
    reservation_days: 90,
  },
};

const RENAMED = {
  status: 200,
  body: {
    claimed_handle: 'NewPilot',
    previous_handle: 'TestPilot',
    token: 'token-for-newpilot',
    verified: false,
    free_change: false,
    cooldown_until: '2026-12-01T00:00:00Z',
    previous_handle_reserved_until: '2027-01-30T00:00:00Z',
  },
};

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

test('the confirm step lists the consequences before anything changes', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('handle-change-confirm', {
      'GET /v1/auth/me': VERIFIED_ME,
      'GET /v1/auth/rsi/handle': STATUS_VERIFIED,
      'POST /v1/auth/rsi/handle': RENAMED,
    }),
  );
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/settings?pane=rsi#rsi');

  const pane = liveIn(page, '.hp-settings');
  await pane.getByLabel('New RSI handle').fill('NewPilot');
  await pane.getByRole('button', { name: 'Change handle…' }).click();

  await expect(pane.getByText('Change your handle from')).toBeVisible({
    timeout: 30_000,
  });
  await expect(pane.getByText(/stays reserved for you for 90 days/)).toBeVisible();
  await expect(pane.getByText(/can't change your handle again for 30 days/)).toBeVisible();
  await expect(pane.getByText(/won't be until you prove it/)).toBeVisible();

  // Opening the confirm step is a read; nothing has been sent yet.
  const before = await getCalls(request);
  expect(
    before.some((c) => c.method === 'POST' && c.path === '/v1/auth/rsi/handle'),
  ).toBe(false);

  await pane
    .getByRole('button', { name: 'Change handle to NewPilot' })
    .click({ timeout: 30_000 });
  await expect(page.getByText(/Handle changed\. Prove it/)).toBeVisible({
    timeout: 30_000,
  });

  const post = (await getCalls(request)).find(
    (c) => c.method === 'POST' && c.path === '/v1/auth/rsi/handle',
  );
  expect(post?.body).toEqual({ new_handle: 'NewPilot' });

  // The session now carries the token the server issued for the new handle.
  const cookies = await page.context().cookies();
  const session = cookies.find((c) => c.value.includes('token-for-newpilot'));
  expect(session, 'session cookie holds the new token').toBeTruthy();
});

test('a refused change says why and keeps the session', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('handle-change-cooldown', {
      'GET /v1/auth/me': VERIFIED_ME,
      'GET /v1/auth/rsi/handle': STATUS_VERIFIED,
      'POST /v1/auth/rsi/handle': {
        status: 429,
        body: { error: 'rename_cooldown', detail: '2026-12-01T00:00:00+00:00' },
      },
    }),
  );
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/settings?pane=rsi&rename=NewPilot#rsi');

  await liveIn(page, '.hp-settings')
    .getByRole('button', { name: 'Change handle to NewPilot' })
    .click({ timeout: 30_000 });
  await expect(page.getByText(/You changed your handle recently/)).toBeVisible({
    timeout: 30_000,
  });
  const cookies = await page.context().cookies();
  expect(cookies.some((c) => c.value.includes('token-for-newpilot'))).toBe(false);
});

test('inside the cooldown the pane says when, and offers no change form', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('handle-change-wait', {
      'GET /v1/auth/me': VERIFIED_ME,
      'GET /v1/auth/rsi/handle': {
        status: 200,
        body: {
          ...STATUS_VERIFIED.body,
          cooldown_until: '2099-12-01T00:00:00Z',
          previous_handle: 'OldPilot',
          previous_handle_reserved_until: '2100-01-30T00:00:00Z',
        },
      },
    }),
  );
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/settings?pane=rsi#rsi');

  const pane = liveIn(page, '.hp-settings');
  await expect(pane.getByText(/You can change it again on/)).toBeVisible({
    timeout: 30_000,
  });
  await expect(pane.getByLabel('New RSI handle')).toHaveCount(0);
});

test('a verified user can re-verify and sees the bio code', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('handle-reverify', {
      'GET /v1/auth/me': VERIFIED_ME,
      'GET /v1/auth/rsi/handle': STATUS_VERIFIED,
      'POST /v1/auth/rsi/reverify': {
        status: 200,
        body: {
          verified: true,
          code: 'SS-VERIFY-REVERIFY1',
          expires_at: '2099-08-23T12:00:00Z',
          claimed_handle: 'TestPilot',
        },
      },
    }),
  );
  await loginAs(page, { handle: 'TestPilot' });
  await page.goto('/settings?pane=rsi#rsi');

  const pane = liveIn(page, '.hp-settings');
  await pane.getByRole('link', { name: 'Re-verify TestPilot' }).click();
  await expect(pane.getByText('SS-VERIFY-REVERIFY1')).toBeVisible({
    timeout: 30_000,
  });
  await expect(pane.getByRole('button', { name: 'Check now' })).toBeVisible();
});

test('a profile link on a renamed-away handle lands on the new one', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('handle-redirect', {
      'GET /v1/public/OldPilot/summary': {
        status: 404,
        body: { error: 'not_found' },
      },
      'GET /v1/handles/OldPilot/current': {
        status: 200,
        body: { handle: 'TestPilot' },
      },
    }),
  );
  await page.goto('/u/OldPilot?range=7d');
  await expect(page).toHaveURL(/\/u\/TestPilot\?range=7d$/, { timeout: 30_000 });
});

test('an unknown handle is not redirected anywhere', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('handle-no-redirect', {
      'GET /v1/public/Nobody/summary': { status: 404, body: { error: 'not_found' } },
      'GET /v1/handles/Nobody/current': { status: 404, body: { error: 'not_found' } },
    }),
  );
  await page.goto('/u/Nobody');
  await expect(page).toHaveURL(/\/u\/Nobody$/);
});
