/**
 * `/chat`: the gates in front of chat, and the way in from /friends.
 *
 * The conversation itself runs in the browser against the homeserver
 * (components/chat/ChatApp), which the mock API does not provide; these
 * specs cover everything StarStats decides. The encrypted round trip was
 * verified against a real Synapse built from infra/synapse.
 */
import { test, expect, type Page } from '@playwright/test';
import { currentUser, loginAs, resetScenario, scenarioFor, setScenario } from './helpers/api-mock';

const status = (over: Record<string, unknown> = {}) => ({
  status: 200,
  body: {
    available: true,
    can_chat: false,
    rsi_verified: true,
    age_declared: false,
    minimum_age: 18,
    restricted: false,
    user_id: '@testpilot:starstats.app',
    homeserver_url: 'http://localhost:1/',
    ...over,
  },
});

const DM_ROOM = '!dm1:starstats.app';

const BASE = {
  'GET /v1/auth/me': currentUser,
  'GET /v1/me/chat': status(),
  'GET /v1/me/chat/rooms': { status: 200, body: { rooms: [] } },
};

test.beforeEach(async ({ page, request }) => {
  await resetScenario(request);
  await setScenario(request, scenarioFor('chat', BASE));
  await loginAs(page, { handle: 'TestPilot' });
});

async function visit(page: Page, query = ''): Promise<void> {
  await page.goto(`/chat${query}`, { timeout: 60_000 });
}

test('chat asks for the age declaration first, and records it', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('chat_age', {
      ...BASE,
      'POST /v1/me/chat/age-declaration': status({ age_declared: true, can_chat: true }),
    }),
  );
  await visit(page);
  const form = page.getByTestId('age-declaration');
  await expect(form).toContainText('aged 18 and over');
  await form.getByLabel('I am 18 or over').check();
  await form.getByRole('button', { name: 'Continue' }).click({ timeout: 30_000 });
  await expect(page).toHaveURL(/\/chat$/, { timeout: 30_000 });
});

test('each closed gate says which one', async ({ page, request }) => {
  for (const [over, text] of [
    [{ rsi_verified: false }, 'Chat needs a verified RSI handle'],
    [{ restricted: true }, 'restricted from chat'],
    [{ available: false }, 'not switched on yet'],
  ] as const) {
    await setScenario(request, scenarioFor('chat_gate', { ...BASE, 'GET /v1/me/chat': status(over) }));
    await visit(page);
    await expect(page.getByText(text)).toBeVisible();
    await expect(page.getByTestId('chat-app')).toHaveCount(0);
  }
});

test('with every gate open, your chats are listed by who or what they are', async ({
  page,
  request,
}) => {
  await setScenario(
    request,
    scenarioFor('chat_rooms', {
      ...BASE,
      'GET /v1/me/chat': status({ age_declared: true, can_chat: true }),
      'GET /v1/me/chat/rooms': {
        status: 200,
        body: {
          rooms: [
            { room_id: DM_ROOM, kind: 'dm', post_id: null, other_handle: 'ssdemowingman', created_at: '2026-09-28T12:00:00Z' },
            { room_id: '!crew1:starstats.app', kind: 'crew', post_id: '0199b000-0000-7000-8000-000000000001', other_handle: null, created_at: '2026-09-28T11:00:00Z' },
          ],
        },
      },
      // The homeserver is unreachable here; the page must say so rather
      // than hang.
      'POST /v1/me/matrix/login-token': {
        status: 200,
        body: { token: 'x', user_id: '@testpilot:starstats.app', homeserver_url: 'http://localhost:1/', expires_in: 60 },
      },
    }),
  );
  await visit(page, `?room=${encodeURIComponent(DM_ROOM)}`);
  const rooms = page.getByRole('navigation', { name: 'Your chats' });
  await expect(rooms.getByRole('button', { name: '@ssdemowingman' })).toHaveAttribute('aria-current', 'true');
  await expect(rooms.getByRole('button', { name: 'Crew chat' })).toBeVisible();
  await expect(page.getByText('StarStats cannot read these messages')).toBeVisible();
  await expect(page.getByRole('alert')).toBeVisible({ timeout: 30_000 });
});

test('a friend has a Message button that opens the DM', async ({ page, request }) => {
  await setScenario(
    request,
    scenarioFor('chat_from_friends', {
      ...BASE,
      'GET /v1/me/friends': {
        status: 200,
        body: {
          friends: [{ handle: 'SSDemoWingman', since: '2026-08-01T12:00:00Z', rsi_verified: true }],
          incoming: [],
          outgoing: [],
          friend_request_policy: 'everyone',
          discoverable: true,
        },
      },
      'POST /v1/me/chat/dm/SSDemoWingman': { status: 200, body: { room_id: DM_ROOM } },
    }),
  );
  await page.goto('/friends', { timeout: 60_000 });
  await page
    .getByTestId('friend-row')
    .filter({ hasText: '@SSDemoWingman' })
    .getByRole('button', { name: 'Message' })
    .click({ timeout: 30_000 });
  await expect(page).toHaveURL(/\/chat\?room=/, { timeout: 30_000 });
});
