import { expect, test } from '@playwright/test';
import {
  currentUser,
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
  unauthorized,
} from './helpers/api-mock';

test.beforeEach(async ({ request, page }) => {
  await resetScenario(request);
  await loginAs(page);
});

test('change_password_success_shows_status', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('settings_pwd_ok', {
      'GET /v1/auth/me': currentUser,
      'POST /v1/auth/me/password': { status: 200, body: { ok: true } },
    }),
  );

  await page.goto('/settings');
  await page.getByLabel('Current password').fill('oldpassword!!');
  await page.getByLabel('New password').fill('newpassword12345');
  await page.getByRole('button', { name: 'Update password' }).click();

  await expect(page).toHaveURL(/\/settings\?status=password_changed/);
  await expect(page.getByText('Password updated.')).toBeVisible();
});

test('change_password_wrong_current_shows_error', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('settings_pwd_bad', {
      'GET /v1/auth/me': currentUser,
      'POST /v1/auth/me/password': unauthorized,
    }),
  );

  await page.goto('/settings');
  await page.getByLabel('Current password').fill('wrongoldpw!!');
  await page.getByLabel('New password').fill('newpassword12345');
  await page.getByRole('button', { name: 'Update password' }).click();

  await expect(page).toHaveURL(/\/settings\?error=invalid_credentials/);
  await expect(page.getByText('Current password is incorrect.')).toBeVisible();
});

test('delete_account_requires_handle_confirmation', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('settings_delete', {
      'GET /v1/auth/me': currentUser,
      'DELETE /v1/auth/me': { status: 200, body: { deleted: true } },
    }),
  );

  await page.goto('/settings');

  // The form requires the handle to match. The server-side action
  // returns 400 confirm_mismatch when the typed handle differs;
  // re-arm the scenario for that path.
  await setScenario(
    request,
    scenarioFor('settings_delete_mismatch', {
      'GET /v1/auth/me': currentUser,
      'DELETE /v1/auth/me': {
        status: 400,
        body: { error: 'confirm_mismatch' },
      },
    }),
  );
  await page.getByLabel('Type your handle to confirm').fill('NotMyHandle');
  await page.getByRole('button', { name: 'Delete my account' }).click();
  await expect(page).toHaveURL(/\/settings\?error=confirm_mismatch/);
  await expect(
    page.getByText("That handle doesn't match. Account was not deleted."),
  ).toBeVisible();

  // Now correct handle path — succeeds and bounces home.
  await setScenario(
    request,
    scenarioFor('settings_delete_ok', {
      'GET /v1/auth/me': currentUser,
      'DELETE /v1/auth/me': { status: 200, body: { deleted: true } },
    }),
  );
  await page.getByLabel('Type your handle to confirm').fill('TestPilot');
  await page.getByRole('button', { name: 'Delete my account' }).click();

  await expect(page).toHaveURL(/\/$/);
  const cookies = await page.context().cookies();
  expect(cookies.find((c) => c.name === 'starstats_session')).toBeUndefined();
});
