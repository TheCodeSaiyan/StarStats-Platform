/**
 * Cross-session entity rollup pages.
 *
 * Asserts:
 *   * `/u/{handle}/entities` renders a grid of entity cards from the
 *     mocked API response.
 *   * Kind filter chips re-render the page with the URL param set
 *     and the visible list narrows.
 *   * `/u/{handle}/entities/{kind}/{id}` renders the per-entity
 *     history with the session breakdown and pagination affordance.
 *   * 403 / forbidden state renders a non-leaky "not available" text.
 *   * Session-page entity section header links to the per-entity
 *     cross-session view.
 */

import { expect, test } from '@playwright/test';
import { loginAs, resetScenario, setScenario } from './helpers/api-mock';

test.beforeEach(async ({ request }) => {
  await resetScenario(request);
});

const HANDLE = 'JohnSomeone';

const cutlassSummary = {
  kind: 'vehicle',
  id: 'CUTLASS_GEID',
  display_name: 'Cutlass Black',
  event_count: 12,
  first_seen: '2026-04-01T00:00:00+00:00',
  last_seen: '2026-05-17T00:00:00+00:00',
  session_count: 3,
};

const auroraSummary = {
  kind: 'vehicle',
  id: 'AURORA_GEID',
  display_name: 'Aurora',
  event_count: 4,
  first_seen: '2026-04-10T00:00:00+00:00',
  last_seen: '2026-05-10T00:00:00+00:00',
  session_count: 2,
};

const playerSummary = {
  kind: 'player',
  id: 'JohnSomeone',
  display_name: 'JohnSomeone',
  event_count: 30,
  first_seen: '2026-04-01T00:00:00+00:00',
  last_seen: '2026-05-17T01:00:00+00:00',
  session_count: 5,
};

test('entities index renders grid of cards with summary data', async ({
  page,
  request,
}) => {
  await loginAs(page, { handle: 'Viewer' });
  await setScenario(request, {
    __id: 'entities_index_full',
    routes: {
      [`GET /v1/users/${HANDLE}/entities`]: {
        status: 200,
        body: {
          entities: [cutlassSummary, auroraSummary, playerSummary],
          next_after: null,
        },
      },
    },
  });

  await page.goto(`/u/${HANDLE}/entities`);

  // The grid carries the data-testid wrapper.
  await expect(page.getByTestId('entities-grid')).toBeVisible();
  const cards = page.getByTestId('entity-card');
  await expect(cards).toHaveCount(3);
  // Each card surfaces the display name + event/session counts.
  await expect(cards.first()).toContainText('Cutlass Black');
  await expect(cards.first()).toContainText('12 events');
  await expect(cards.first()).toContainText('3 sessions');
});

test('entities index filter narrows the visible cards', async ({
  page,
  request,
}) => {
  await loginAs(page, { handle: 'Viewer' });
  // The Next server fetches `/v1/users/.../entities` on every navigation
  // because the page is a server component. The mock returns the same
  // body for both the initial render and the filtered-URL render — the
  // client-side filter narrows from the same superset.
  await setScenario(request, {
    __id: 'entities_index_filter',
    routes: {
      [`GET /v1/users/${HANDLE}/entities`]: {
        status: 200,
        body: {
          entities: [cutlassSummary, auroraSummary, playerSummary],
          next_after: null,
        },
      },
    },
  });

  await page.goto(`/u/${HANDLE}/entities`);
  await expect(page.getByTestId('entity-card')).toHaveCount(3);

  // Click the Player filter chip; only the player card should remain.
  await page.getByTestId('entities-filter-player').click();
  await expect(page).toHaveURL(/[?&]kind=player(&|$)/);
  const remaining = page.getByTestId('entity-card');
  await expect(remaining).toHaveCount(1);
  await expect(remaining.first()).toHaveAttribute('data-kind', 'player');
});

test('entities index forbidden state shows non-leaky message', async ({
  page,
  request,
}) => {
  await loginAs(page, { handle: 'Viewer' });
  await setScenario(request, {
    __id: 'entities_index_forbidden',
    routes: {
      [`GET /v1/users/${HANDLE}/entities`]: {
        status: 403,
        body: { error: 'share_event_timeline_not_granted' },
      },
    },
  });

  await page.goto(`/u/${HANDLE}/entities`);

  await expect(page.getByTestId('entities-forbidden')).toBeVisible();
  await expect(page.locator('body')).toContainText(
    /hasn.t shared their event history/i,
  );
  // The 403 reason string must not appear verbatim — the UI summarises.
  await expect(page.locator('body')).not.toContainText('grant');
  await expect(page.locator('body')).not.toContainText('forbidden');
});

const cutlassEvent1 = {
  idempotency_key: 'k1',
  raw_line: '<2026-04-01T00:00:00.000Z> [VehicleDestruction]',
  source: 'live',
  source_offset: 10,
  event: {
    type: 'vehicle_destruction',
    timestamp: '2026-04-01T00:00:00.000Z',
    vehicle: 'CUTLASS_GEID',
    vehicle_class: 'Cutlass',
    destroy_level: 2,
    cause_player: null,
    cause_geid: null,
    damage_type: null,
    zone: null,
  },
  metadata: {
    primary_entity: {
      kind: 'vehicle',
      id: 'CUTLASS_GEID',
      display_name: 'Cutlass Black',
    },
    source: 'observed',
    confidence: 1.0,
    group_key: 'vehicle_destruction:vehicle:CUTLASS_GEID',
  },
};

const cutlassEvent2 = {
  ...cutlassEvent1,
  idempotency_key: 'k2',
  source_offset: 20,
  raw_line: '<2026-05-17T00:00:00.000Z> [VehicleDestruction]',
  event: {
    ...cutlassEvent1.event,
    timestamp: '2026-05-17T00:00:00.000Z',
  },
};

test('per-entity page renders history with session breakdown', async ({
  page,
  request,
}) => {
  await loginAs(page, { handle: 'Viewer' });
  await setScenario(request, {
    __id: 'entity_history_full',
    routes: {
      [`GET /v1/users/${HANDLE}/entities/vehicle/CUTLASS_GEID`]: {
        status: 200,
        body: {
          kind: 'vehicle',
          id: 'CUTLASS_GEID',
          display_name: 'Cutlass Black',
          events: [cutlassEvent1, cutlassEvent2],
          next_after: null,
          session_breakdown: [
            {
              session_id: 'session-1',
              started_at: '2026-04-01T00:00:00+00:00',
              event_count: 1,
            },
            {
              session_id: 'session-2',
              started_at: '2026-05-17T00:00:00+00:00',
              event_count: 1,
            },
          ],
        },
      },
    },
  });

  await page.goto(`/u/${HANDLE}/entities/vehicle/CUTLASS_GEID`);

  await expect(page.getByRole('heading', { level: 1 })).toContainText(
    'Cutlass Black',
  );
  // Stats trio surfaces event + session counts.
  const stats = page.getByTestId('entity-stats');
  await expect(stats).toContainText('2 events');
  await expect(stats).toContainText('2 sessions');

  // Session breakdown lists both sessions, each linking to the
  // per-session timeline.
  const buckets = page.getByTestId('entity-session-bucket');
  await expect(buckets).toHaveCount(2);
  await expect(buckets.first()).toContainText('session-1');

  // Both events fold into one chronological row (same group_key).
  await expect(page.getByTestId('timeline-row')).toHaveCount(1);
  await expect(page.getByText('×2')).toBeVisible();
});

test('per-entity page exposes load-more when next_after is present', async ({
  page,
  request,
}) => {
  await loginAs(page, { handle: 'Viewer' });
  await setScenario(request, {
    __id: 'entity_history_paginated',
    routes: {
      [`GET /v1/users/${HANDLE}/entities/vehicle/CUTLASS_GEID`]: {
        status: 200,
        body: {
          kind: 'vehicle',
          id: 'CUTLASS_GEID',
          display_name: 'Cutlass Black',
          events: [cutlassEvent1],
          next_after: 'k1',
          session_breakdown: [],
        },
      },
    },
  });

  await page.goto(`/u/${HANDLE}/entities/vehicle/CUTLASS_GEID`);
  const loadMore = page.getByTestId('entity-history-load-more');
  await expect(loadMore).toBeVisible();
  const href = await loadMore.getAttribute('href');
  expect(href).toContain('after=k1');
});

test('session-page entity section links to per-entity cross-session page', async ({
  page,
  request,
}) => {
  await loginAs(page, { handle: 'Viewer' });
  const sessionId = 'session-abc';
  await setScenario(request, {
    __id: 'entity_cross_link_from_session',
    routes: {
      [`GET /v1/users/${HANDLE}/sessions/${sessionId}/events`]: {
        status: 200,
        body: {
          session_id: sessionId,
          events: [cutlassEvent1],
          next_after: null,
        },
      },
    },
  });

  await page.goto(`/u/${HANDLE}/sessions/${sessionId}`);

  const sectionLink = page.getByTestId('entity-section-link').first();
  await expect(sectionLink).toBeVisible();
  const href = await sectionLink.getAttribute('href');
  expect(href).toBe(
    `/u/${HANDLE}/entities/vehicle/CUTLASS_GEID`,
  );
  // The aria-label spells out the cross-session intent.
  const aria = await sectionLink.getAttribute('aria-label');
  expect(aria).toContain('across sessions');
});
