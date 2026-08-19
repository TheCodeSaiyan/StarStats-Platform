/**
 * Per-event session timeline page.
 *
 * Asserts:
 *   * Entity-grouped sections render when metadata is present.
 *   * The `InferredBadge` renders next to events with
 *     `metadata.source === 'inferred'`.
 *   * No submission/redaction UI is exposed (tray-only affordance).
 *   * Forbidden / missing access collapses to a single "not available"
 *     state — we never leak whether the session exists.
 */

import { expect, test } from '@playwright/test';
import { loginAs, resetScenario, setScenario } from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

const HANDLE = 'JohnSomeone';
const SESSION_ID = 'session-abc';

const observedDeath = {
  idempotency_key: 'idem-1',
  raw_line: '<2026-05-17T00:00:00.000Z> [Death] PlayerDeath',
  source: 'live',
  source_offset: 10,
  event: {
    type: 'player_death',
    timestamp: '2026-05-17T00:00:00.000Z',
    body_class: 'body_01_noMagicPocket',
    body_id: '1',
    zone: null,
  },
  metadata: {
    primary_entity: {
      kind: 'player',
      id: 'JohnSomeone',
      display_name: 'JohnSomeone',
    },
    source: 'observed',
    confidence: 1.0,
    group_key: 'player_death:player:JohnSomeone',
  },
};

const inferredDeath = {
  idempotency_key: 'idem-2',
  raw_line: '<2026-05-17T00:05:00.000Z> [Inferred] ImplicitDeath',
  source: 'live',
  source_offset: 20,
  event: {
    type: 'player_death',
    timestamp: '2026-05-17T00:05:00.000Z',
    body_class: 'body_02_other',
    body_id: '2',
    zone: null,
  },
  metadata: {
    primary_entity: {
      kind: 'player',
      id: 'JohnSomeone',
      display_name: 'JohnSomeone',
    },
    source: 'inferred',
    confidence: 0.85,
    group_key: 'player_death:player:JohnSomeone',
  },
};

const sessionEventsBody = {
  session_id: SESSION_ID,
  events: [observedDeath, inferredDeath],
  next_after: null,
};

test('session page renders entity-grouped timeline with inferred badge', async ({
  page,
  request,
}) => {
  await loginAs(page, { handle: 'Viewer' });
  await setScenario(request, {
    __id: 'session_timeline_entity_view',
    routes: {
      [`GET /v1/users/${HANDLE}/sessions/${SESSION_ID}/events`]: {
        status: 200,
        body: sessionEventsBody,
      },
    },
  });

  await page.goto(`/u/${HANDLE}/sessions/${SESSION_ID}`);

  // Header confirms session id.
  await expect(
    page.getByRole('heading', { level: 1 }),
  ).toContainText(SESSION_ID);

  // Entity section renders for the player.
  const sections = page.getByTestId('entity-section');
  await expect(sections).toHaveCount(1);
  await expect(sections.first()).toContainText('JohnSomeone');

  // Inferred badge rendered for the inferred row. The badge carries
  // an aria-label that starts with "Inferred event"; matching by the
  // accessible label keeps the selector resilient to the underlying
  // tag/role choice.
  const inferredBadges = page.getByLabel(/inferred event/i);
  await expect(inferredBadges.first()).toBeVisible();
  await expect(inferredBadges).toHaveCount(1);

  // Same `group_key` -> the two events fold into one row with a
  // count badge. The badge is the `×N` mono span the row renders
  // alongside the title.
  await expect(page.getByTestId('timeline-row')).toHaveCount(1);
  await expect(page.getByText('×2')).toBeVisible();
});

test('session page exposes no submission UI', async ({ page, request }) => {
  await loginAs(page, { handle: 'Viewer' });
  await setScenario(request, {
    __id: 'session_timeline_no_submission_ui',
    routes: {
      [`GET /v1/users/${HANDLE}/sessions/${SESSION_ID}/events`]: {
        status: 200,
        body: sessionEventsBody,
      },
    },
  });

  await page.goto(`/u/${HANDLE}/sessions/${SESSION_ID}`);

  // Wait for the timeline to render so the absence-check has a stable
  // page to inspect.
  await expect(page.getByTestId('timeline-row').first()).toBeVisible();

  // Parser-submission affordances are tray-only; the per-event web
  // page must NOT expose them. Be exhaustive across roles.
  await expect(page.getByRole('button', { name: /submit/i })).toHaveCount(0);
  await expect(page.getByRole('link', { name: /submit/i })).toHaveCount(0);
  await expect(page.getByRole('button', { name: /redact/i })).toHaveCount(0);
  await expect(page.getByRole('button', { name: /hide/i })).toHaveCount(0);
});

test('forbidden session collapses to a generic not-available state', async ({
  page,
  request,
}) => {
  await loginAs(page, { handle: 'Viewer' });
  await setScenario(request, {
    __id: 'session_timeline_forbidden',
    routes: {
      [`GET /v1/users/${HANDLE}/sessions/${SESSION_ID}/events`]: {
        status: 403,
        body: { error: 'share_event_timeline_not_granted' },
      },
    },
  });

  await page.goto(`/u/${HANDLE}/sessions/${SESSION_ID}`);

  await expect(
    page.getByRole('heading', { name: /session not available/i }),
  ).toBeVisible();
  // Don't leak whether the session id was real — error text stays
  // generic.
  await expect(page.locator('body')).not.toContainText('grant');
  await expect(page.locator('body')).not.toContainText('forbidden');
});

test('empty session renders an empty-state card', async ({ page, request }) => {
  await loginAs(page, { handle: 'Viewer' });
  await setScenario(request, {
    __id: 'session_timeline_empty',
    routes: {
      [`GET /v1/users/${HANDLE}/sessions/${SESSION_ID}/events`]: {
        status: 200,
        body: {
          session_id: SESSION_ID,
          events: [],
          next_after: null,
        },
      },
    },
  });

  await page.goto(`/u/${HANDLE}/sessions/${SESSION_ID}`);
  await expect(page.getByText(/session has no events/i)).toBeVisible();
});
