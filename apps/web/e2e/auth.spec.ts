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

test('signup_success_redirects_to_emitter', async ({ page, request }) => {
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

  // The signup action redirects to the Emitter on success — a new account's
  // first job is pairing a client.
  await expect(page).toHaveURL(/\/downloads/);
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

  await expect(page).toHaveURL(/\/downloads/);
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
  // /me; only signup → /downloads for new-user pairing onboarding.)
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
  // Scoped to the form. `/auth/**` is a projection now, and its `ChromeBar`
  // offers its own "Sign in" action for a signed-out visitor — so an unscoped
  // locator matches two buttons.
  await page.locator('.hp-auth').getByRole('button', { name: 'Sign in' }).click();

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
  // Scoped to the form. `/auth/**` is a projection now, and its `ChromeBar`
  // offers its own "Sign in" action for a signed-out visitor — so an unscoped
  // locator matches two buttons.
  await page.locator('.hp-auth').getByRole('button', { name: 'Sign in' }).click();

  await expect(page).toHaveURL(/\/auth\/login\?error=invalid_credentials/);
  await expect(page.getByText('Email or password is incorrect.')).toBeVisible();
});

test('logout_clears_session_redirects_home', async ({ page, request }) => {
  await setScenario(request, scenarioFor('logout'));
  await loginAs(page, { handle: 'TestPilot' });

  // Visit any logged-in page first to confirm cookie-based auth works.
  // The heading is "Emitter", not "Devices": the design system uses
  // in-universe nouns for chrome (email → comm-link, the desktop client →
  // emitter), and `/devices` was folded into `/downloads` so the client's
  // whole lifecycle — download, pair, watch, revoke — is one destination.
  await page.goto('/downloads');
  await expect(
    page.getByRole('heading', { name: 'Emitter', exact: true, level: 1 }),
  ).toBeVisible();

  // Hitting the logout route clears the cookie and bounces home.
  await page.goto('/auth/logout');
  await expect(page).toHaveURL(/\/$/);

  const cookies = await page.context().cookies();
  const session = cookies.find((c) => c.name === 'starstats_session');
  expect(session).toBeUndefined();
});

/* -----------------------------------------------------------------------------
   The auth flow's chrome. These are NOT capture specs — they guard two things
   that a green suite otherwise says nothing about.
   -------------------------------------------------------------------------- */

test('auth_pane_names_the_step_not_the_section', async ({ page }) => {
  // Before this, all nine `/auth/**` routes shared one header reading
  // "Access" — the same words whether you were signing in, resetting a
  // passphrase or confirming an address. Asserting one page's title would
  // pass on that; the assertion has to be that two steps read DIFFERENTLY.
  await page.goto('/auth/login');
  const loginTitle = await page.locator('.hp-phd h2').first().textContent();
  // `innerText` would come back "SIGN IN" — the pane header is uppercased in
  // CSS. Read `textContent` so this asserts the CONTENT, and check the casing
  // separately below so the two failures stay distinguishable.
  const loginRendered = await page
    .locator('.hp-phd h2')
    .first()
    .innerText();

  await page.goto('/auth/forgot-password');
  const resetTitle = await page.locator('.hp-phd h2').first().textContent();

  expect(loginTitle?.trim()).toBe('Sign in');
  expect(resetTitle?.trim()).toBe('Reset passphrase');
  expect(loginTitle).not.toBe(resetTitle);
  // The pane header idiom is tracked uppercase; if this comes back mixed-case
  // the header is rendering unstyled.
  expect(loginRendered.trim()).toBe('SIGN IN');
});

test('auth_page_keeps_exactly_one_h1', async ({ page }) => {
  // The projection's crumb can carry an h1 and these pages bring their own —
  // state-specific, which is the better heading. Only one of the two may win.
  for (const route of ['/auth/login', '/auth/signup', '/auth/magic-link']) {
    await page.goto(route);
    await expect(page.locator('h1'), route).toHaveCount(1);
  }
});

test('auth_ways_in_navigate_without_a_reload', async ({ page }) => {
  await page.goto('/auth/login');

  // A full document load would clear this. Same probe the chrome-nav guard
  // uses: the seam this catches is a plain <a> where a Link belongs.
  await page.evaluate(() => {
    (window as unknown as Record<string, unknown>).__authNoReload = true;
  });

  const strip = page.getByRole('navigation', { name: 'Ways in' });
  await strip.getByRole('link', { name: 'Magic link', exact: true }).click();

  await expect(page).toHaveURL(/\/auth\/magic-link$/);
  await expect(page.locator('.hp-phd h2').first()).toHaveText('Magic link', {
    useInnerText: false,
  });
  expect(
    await page.evaluate(
      () => (window as unknown as Record<string, unknown>).__authNoReload,
    ),
  ).toBe(true);
});

test('auth_ways_in_offers_no_token_gated_route', async ({ page }) => {
  // Listing `reset-password`, `verify` or the magic-link redeem would send a
  // reader to an error state and call it a destination.
  await page.goto('/auth/login');
  const hrefs = await page
    .getByRole('navigation', { name: 'Ways in' })
    .getByRole('link')
    .evaluateAll((els) => els.map((e) => e.getAttribute('href')));

  expect(hrefs.length).toBeGreaterThan(0);
  for (const href of hrefs) {
    expect(href).not.toMatch(
      /reset-password|\/verify|email-change|totp-verify|redeem/,
    );
  }
});

test('auth_body_is_calibrated_not_flat', async ({ page }) => {
  // The nine auth pages used to set their own type inline — 28px/600, which
  // is the flat scale, not the beam's. Inline styles are the one thing the
  // projection's redraw cannot reach, so this reads the COMPUTED weight: a
  // regression here looks fine in review and wrong on screen.
  await page.goto('/auth/login');
  const h1 = page.locator('h1').first();
  await expect(h1).toBeVisible();
  const weight = await h1.evaluate((el) => getComputedStyle(el).fontWeight);
  expect(Number(weight)).toBeLessThanOrEqual(400);
});
