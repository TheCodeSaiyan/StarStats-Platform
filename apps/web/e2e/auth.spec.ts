import { expect, test } from '@playwright/test';
import {
  conflict,
  currentUser,
  getCalls,
  loginAs,
  resetScenario,
  scenarioFor,
  setScenario,
  successfulLogin,
  successfulSignup,
  unauthorized,
} from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

test('signup_success_redirects_to_devices', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('signup_success', {
      'POST /v1/auth/signup': successfulSignup,
      'GET /v1/auth/me': currentUser,
    }),
  );

  await page.goto('/auth/signup');
  await page.getByLabel('Email').fill('pilot@example.test');
  await page.getByLabel('Password').fill('thisisapasswordy');
  await page.getByLabel('RSI handle').fill('TestPilot');
  await page.getByRole('button', { name: 'Create account' }).click();

  // The signup action redirects to /devices on success.
  await expect(page).toHaveURL(/\/devices/);
});

test('signup_with_existing_email_shows_error', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('signup_email_taken', {
      'POST /v1/auth/signup': conflict('email_taken'),
    }),
  );

  await page.goto('/auth/signup');
  await page.getByLabel('Email').fill('taken@example.test');
  await page.getByLabel('Password').fill('thisisapasswordy');
  await page.getByLabel('RSI handle').fill('TestPilot');
  await page.getByRole('button', { name: 'Create account' }).click();

  // Action redirects back to /auth/signup?error=email_taken on the
  // 409 case; the page renders the friendly label.
  await expect(page).toHaveURL(/\/auth\/signup\?error=email_taken/);
  await expect(
    page.getByText('An account with that email already exists.'),
  ).toBeVisible();
});

test('signup_invite_link_preserves_the_invite_token', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('signup_with_invite', {
      'GET /v1/waitlist/status': { status: 200, body: { gate_enabled: true } },
      'POST /v1/auth/signup': successfulSignup,
      'GET /v1/auth/me': currentUser,
    }),
  );
  await page.context().addCookies([
    {
      name: 'ss_beta_dismissed',
      value: '1',
      domain: 'localhost',
      path: '/',
    },
  ]);

  await page.goto('/auth/signup?invite=invite-demo-123');
  await page.getByLabel('Email').fill('pilot@example.test');
  await page.getByLabel('Password').fill('thisisapasswordy');
  await page.getByLabel('RSI handle').fill('TestPilot');
  await page.getByRole('button', { name: 'Create account' }).click();

  await expect(page).toHaveURL(/\/devices/);
  const calls = await getCalls(request);
  const signupCall = calls.find(
    (call) => call.method === 'POST' && call.path === '/v1/auth/signup',
  );
  expect(signupCall?.body).toMatchObject({
    email: 'pilot@example.test',
    claimed_handle: 'TestPilot',
    invite_token: 'invite-demo-123',
  });
});

test('login_success_redirects_to_me', async ({ page, request }) => {
  // The login server action redirects to /me (the home mirror) on
  // success — see src/app/auth/login/page.tsx. (Sign-in flows land on
  // /me; only signup → /devices for new-user pairing onboarding.)
  await setScenario(
    request,
    scenarioFor('login_success', {
      'POST /v1/auth/login': successfulLogin,
      'GET /v1/auth/me': currentUser,
    }),
  );

  await page.goto('/auth/login');
  await page.getByLabel('Email').fill('pilot@example.test');
  await page.getByLabel('Password').fill('thisisapasswordy');
  await page.getByRole('button', { name: 'Sign in' }).click();

  await expect(page).toHaveURL(/\/me/);
});

test('login_with_wrong_password_shows_error', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('login_bad_password', {
      'POST /v1/auth/login': unauthorized,
    }),
  );

  await page.goto('/auth/login');
  await page.getByLabel('Email').fill('pilot@example.test');
  await page.getByLabel('Password').fill('wrongpassword!!');
  await page.getByRole('button', { name: 'Sign in' }).click();

  await expect(page).toHaveURL(/\/auth\/login\?error=invalid_credentials/);
  await expect(page.getByText('Email or password is incorrect.')).toBeVisible();
});

test('logout_clears_session_redirects_home', async ({ page, request }) => {
  await setScenario(request, scenarioFor('logout'));
  await loginAs(page, { handle: 'TestPilot' });

  // Visit any logged-in page first to confirm cookie-based auth works.
  await page.goto('/devices');
  await expect(
    page.getByRole('heading', { name: 'Devices', exact: true }),
  ).toBeVisible();

  // Hitting the logout route clears the cookie and bounces home.
  await page.goto('/auth/logout');
  await expect(page).toHaveURL(/\/$/);

  const cookies = await page.context().cookies();
  const session = cookies.find((c) => c.name === 'starstats_session');
  expect(session).toBeUndefined();
});
