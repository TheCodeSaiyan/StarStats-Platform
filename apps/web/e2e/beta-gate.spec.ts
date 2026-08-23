import { expect, test } from '@playwright/test';
import {
  getCalls,
  resetScenario,
  scenarioFor,
  setScenario,
} from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

// Gate OFF (default) → no overlay: this is the dormant-ship guarantee.
test('beta_gate_hidden_when_gate_off', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('beta_gate_off', {
      'GET /v1/waitlist/status': { status: 200, body: { gate_enabled: false } },
    })
  );
  await page.goto('/');
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

// Gate ON + no dismiss cookie → overlay visible with the join form.
test('beta_gate_shown_when_gate_on', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('beta_gate_on', {
      'GET /v1/waitlist/status': { status: 200, body: { gate_enabled: true } },
    })
  );
  await page.goto('/');
  await expect(page.getByRole('dialog')).toBeVisible();
  await expect(
    page.getByRole('button', { name: /join the waitlist/i })
  ).toBeVisible();
});

// Dismiss → gone, and stays gone on next navigation (cookie).
test('beta_gate_dismiss_is_remembered', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('beta_gate_dismiss', {
      'GET /v1/waitlist/status': { status: 200, body: { gate_enabled: true } },
    })
  );
  await page.goto('/');
  await page.getByRole('button', { name: /browse the site/i }).click();
  await expect(page.getByRole('dialog')).toHaveCount(0);
  await page.goto('/features');
  await expect(page.getByRole('dialog')).toHaveCount(0);
});

test('gated signup offers the waitlist instead of an unusable account form', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('beta_signup_waitlist', {
      'GET /v1/waitlist/status': { status: 200, body: { gate_enabled: true } },
      'POST /v1/waitlist': {
        status: 200,
        body: { joined: true, position: null },
      },
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

  await page.goto('/auth/signup');

  await expect(
    page.getByRole('heading', { name: /join the starstats beta/i }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: /join the waitlist/i }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: /create account/i }),
  ).toHaveCount(0);

  await page.getByLabel('Waitlist email').fill('pilot@example.test');
  await page.getByRole('button', { name: /join the waitlist/i }).click();
  await expect(page.getByText(/signup link is on its way/i)).toBeVisible();
  const calls = await getCalls(request);
  const waitlistCall = calls.find(
    (call) => call.method === 'POST' && call.path === '/v1/waitlist',
  );
  expect(waitlistCall?.body).toMatchObject({ source: 'auth-signup' });
});

test('gated login keeps sign-in and provides a working waitlist form', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('beta_login_waitlist', {
      'GET /v1/waitlist/status': { status: 200, body: { gate_enabled: true } },
      'POST /v1/waitlist': {
        status: 200,
        body: { joined: true, position: 9 },
      },
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

  await page.goto('/auth/login');

  // Scoped to the form. `/auth/**` is a projection now and its `ChromeBar`
  // offers its own "Sign in" for a signed-out visitor, so an unscoped locator
  // matches two buttons.
  await expect(
    page.locator('.hp-auth').getByRole('button', { name: 'Sign in' }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: /join the waitlist/i }),
  ).toBeVisible();
  await page.getByLabel('Waitlist email').fill('pilot@example.test');
  await page.getByRole('button', { name: /join the waitlist/i }).click();
  await expect(page.getByText(/number 9 in the queue/i)).toBeVisible();
  const calls = await getCalls(request);
  const waitlistCall = calls.find(
    (call) => call.method === 'POST' && call.path === '/v1/waitlist',
  );
  expect(waitlistCall?.body).toMatchObject({ source: 'auth-login' });
});

test('auth waitlist surfaces disappear when the beta gate is off', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('auth_waitlist_gate_off', {
      'GET /v1/waitlist/status': { status: 200, body: { gate_enabled: false } },
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

  await page.goto('/auth/signup');
  await expect(
    page.getByRole('button', { name: /create account/i }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: /join the waitlist/i }),
  ).toHaveCount(0);

  await page.goto('/auth/login');
  // Scoped to the form. `/auth/**` is a projection now and its `ChromeBar`
  // offers its own "Sign in" for a signed-out visitor, so an unscoped locator
  // matches two buttons.
  await expect(
    page.locator('.hp-auth').getByRole('button', { name: 'Sign in' }),
  ).toBeVisible();
  await expect(
    page.getByRole('button', { name: /join the waitlist/i }),
  ).toHaveCount(0);
});
