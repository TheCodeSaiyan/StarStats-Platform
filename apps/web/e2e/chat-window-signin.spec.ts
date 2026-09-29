/**
 * The tray's chat window signs in through a one-use magic link with
 * `next=/chat` (POST /v1/me/chat/web-session mints it). These pin where
 * each sign-in lands: chat when asked, /me otherwise, and never anywhere
 * a crafted `next` names. Two-factor still applies and keeps the landing.
 */
import { test, expect } from '@playwright/test';
import {
  currentUser,
  resetScenario,
  scenarioFor,
  setScenario,
  successfulLogin,
} from './helpers/api-mock';

const CHAT = {
  'GET /v1/auth/me': currentUser,
  'GET /v1/me/chat': {
    status: 200,
    body: {
      available: true,
      can_chat: true,
      rsi_verified: true,
      age_declared: true,
      minimum_age: 18,
      restricted: false,
      user_id: '@testpilot:starstats.app',
      homeserver_url: 'http://localhost:1/',
      offered: true,
    },
  },
  'GET /v1/me/chat/rooms': { status: 200, body: { rooms: [] } },
};

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

test('a chat-window sign-in lands in chat', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('chat_signin', { ...CHAT, 'POST /v1/auth/magic/redeem': successfulLogin }),
  );
  await page.goto('/auth/magic-link/redeem?token=abc&next=%2Fchat', { timeout: 60_000 });
  await expect(page).toHaveURL(/\/chat$/, { timeout: 30_000 });
});

for (const next of ['', '&next=%2Fadmin', '&next=https%3A%2F%2Fevil.example%2Fchat']) {
  test(`a sign-in link with next="${next}" lands on /me`, async ({ page, request }) => {
    await setScenario(
      request,
      scenarioFor('chat_signin_me', { ...CHAT, 'POST /v1/auth/magic/redeem': successfulLogin }),
    );
    await page.goto(`/auth/magic-link/redeem?token=abc${next}`, { timeout: 60_000 });
    await expect(page).toHaveURL(/\/me(\?|$)/, { timeout: 30_000 });
  });
}

test('two-factor keeps a chat-window sign-in headed for chat', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('chat_signin_totp', {
      ...CHAT,
      'POST /v1/auth/magic/redeem': {
        status: 200,
        body: { ...successfulLogin.body, token: 'interim-token', totp_required: true },
      },
      'POST /v1/auth/totp/verify-login': successfulLogin,
    }),
  );
  await page.goto('/auth/magic-link/redeem?token=abc&next=%2Fchat', { timeout: 60_000 });
  await expect(page).toHaveURL(/\/auth\/totp-verify\?interim=interim-token&next=%2Fchat/, {
    timeout: 30_000,
  });
  for (let i = 0; i < 6; i++) {
    await page.locator(`input[name="c${i}"]`).fill(String(i + 1));
  }
  await page.getByRole('button', { name: /verify/i }).click({ timeout: 30_000 });
  await expect(page).toHaveURL(/\/chat$/, { timeout: 30_000 });
});

test('an expired or used sign-in link says so', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('magic_invalid', {
      'POST /v1/auth/magic/redeem': { status: 401, body: { error: 'invalid_or_expired' } },
    }),
  );
  await page.goto('/auth/magic-link/redeem?token=stale', { timeout: 60_000 });
  await expect(page.getByRole('heading', { name: 'Sign-in link invalid or expired.' })).toBeVisible({
    timeout: 30_000,
  });
});

test('a sign-in link without a token says so', async ({ page }) => {
  await page.goto('/auth/magic-link/redeem', { timeout: 60_000 });
  await expect(page.getByRole('heading', { name: 'Missing sign-in token.' })).toBeVisible({
    timeout: 30_000,
  });
});
