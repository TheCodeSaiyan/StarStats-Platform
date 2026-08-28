/**
 * Helpers for talking to the local mock API server.
 *
 * Every test posts a fresh "scenario" to the mock before navigating.
 * The scenario is just a `Record<"METHOD path", ResponseStub>` — keys
 * are matched exactly against the incoming Node fetch from the Next
 * dev server.
 *
 * Most fixtures here are intentionally small and synthetic; they
 * mirror the OpenAPI shapes from `packages/api-client-ts/src/generated/schema.ts`
 * but don't try to be exhaustive. Each test extends/overrides only
 * what it cares about.
 */

import type { APIRequestContext, Page } from '@playwright/test';

export const MOCK_PORT = 3199;
export const WEB_PORT = 3100;
export const MOCK_BASE = `http://127.0.0.1:${MOCK_PORT}`;

export interface ResponseStub {
  status?: number;
  body?: unknown;
  /** Hold the response open this many ms before sending. Lets a test
   *  observe a server-rendered `loading.tsx` fallback, which is only on
   *  screen while an upstream call is outstanding. */
  delayMs?: number;
}

export type ScenarioRoutes = Record<string, ResponseStub>;

export interface Scenario {
  __id?: string;
  routes: ScenarioRoutes;
}

/**
 * POST a scenario to the mock server. Call this in `test.beforeEach`
 * (or inline before `page.goto`) so the mock answers with the shapes
 * the assertions expect.
 */
export async function setScenario(
  request: APIRequestContext,
  scenario: Scenario,
): Promise<void> {
  const resp = await request.post(`${MOCK_BASE}/__mock/scenario`, {
    data: scenario,
  });
  if (!resp.ok()) {
    throw new Error(`setScenario failed: ${resp.status()} ${await resp.text()}`);
  }
}

export async function resetScenario(request: APIRequestContext): Promise<void> {
  await request.post(`${MOCK_BASE}/__mock/reset`);
}

/**
 * Read the call log from the mock. Useful for asserting the dashboard
 * issued a request with the right query string, etc.
 */
export async function getCalls(
  request: APIRequestContext,
): Promise<
  Array<{ method: string; path: string; query: string; body: unknown }>
> {
  const resp = await request.get(`${MOCK_BASE}/__mock/calls`);
  const data = (await resp.json()) as {
    calls: Array<{ method: string; path: string; query: string; body: unknown }>;
  };
  return data.calls;
}

/**
 * Set the session cookie directly so the test starts "logged in"
 * without having to walk through the auth flow. Mirrors the cookie
 * shape minted by `setSession()` in `src/lib/session.ts`.
 */
export async function loginAs(
  page: Page,
  opts: {
    token?: string;
    userId?: string;
    handle?: string;
    emailVerified?: boolean;
    /// Site-wide staff grants. Empty by default — tests opt in to
    /// admin / moderator routes by passing `['admin']` or
    /// `['moderator']`. Mirrors `staffRoles` in the Session type.
    staffRoles?: string[];
  } = {},
): Promise<void> {
  const value = JSON.stringify({
    t: opts.token ?? 'test-token',
    u: opts.userId ?? 'user_test',
    h: opts.handle ?? 'TestPilot',
    v: opts.emailVerified ?? true,
    r: opts.staffRoles ?? [],
  });
  await page.context().addCookies([
    {
      name: 'starstats_session',
      value,
      domain: 'localhost',
      path: '/',
      httpOnly: true,
      sameSite: 'Lax',
    },
  ]);
}

// ---------------------------------------------------------------------
// Fixtures — concrete shapes that match the OpenAPI components.
// ---------------------------------------------------------------------

export const successfulSignup = {
  status: 200,
  body: {
    token: 'jwt.signup.token',
    user_id: 'user_new',
    claimed_handle: 'TestPilot',
  },
};

export const successfulLogin = {
  status: 200,
  body: {
    token: 'jwt.login.token',
    user_id: 'user_existing',
    claimed_handle: 'TestPilot',
  },
};

export const currentUser = {
  status: 200,
  body: {
    user_id: 'user_existing',
    email: 'pilot@example.test',
    email_verified: true,
    claimed_handle: 'TestPilot',
  },
};

export const currentUserUnverified = {
  status: 200,
  body: {
    ...currentUser.body,
    email_verified: false,
  },
};

export const summaryWithEvents = {
  status: 200,
  body: {
    claimed_handle: 'TestPilot',
    total: 1234,
    by_type: [
      { event_type: 'login', count: 600 },
      { event_type: 'mission_complete', count: 400 },
      { event_type: 'death', count: 234 },
    ],
  },
};

export const emptySummary = {
  status: 200,
  body: {
    claimed_handle: 'TestPilot',
    total: 0,
    by_type: [],
  },
};

export const timeline30Days = {
  status: 200,
  body: {
    days: 30,
    buckets: Array.from({ length: 30 }, (_, i) => ({
      date: `2026-04-${String(i + 1).padStart(2, '0')}`,
      count: i % 5 === 0 ? 0 : (i + 1) * 3,
    })),
  },
};

/**
 * 50-event page so the dashboard's pager renders an "Older →" link.
 */
export const eventsPageDescending = {
  status: 200,
  body: {
    events: Array.from({ length: 50 }, (_, i) => {
      const seq = 100 - i;
      return {
        seq,
        source_offset: seq * 1024,
        log_source: 'live',
        event_type: i % 2 === 0 ? 'login' : 'mission_complete',
        event_timestamp: '2026-05-04T12:00:00Z',
        payload: { type: i % 2 === 0 ? 'login' : 'mission_complete' },
      };
    }),
    next_after: null,
  },
};

export const eventsFilteredLogin = {
  status: 200,
  body: {
    events: [
      {
        seq: 100,
        source_offset: 102400,
        log_source: 'live',
        event_type: 'login',
        event_timestamp: '2026-05-04T12:00:00Z',
        payload: { type: 'login' },
      },
      {
        seq: 98,
        source_offset: 100352,
        log_source: 'live',
        event_type: 'login',
        event_timestamp: '2026-05-04T11:30:00Z',
        payload: { type: 'login' },
      },
    ],
    next_after: null,
  },
};

export const deviceList = {
  status: 200,
  body: {
    devices: [
      {
        id: 'dev_1',
        label: "Daisy's PC",
        created_at: '2026-04-01T08:00:00Z',
        last_seen_at: '2026-05-04T07:00:00Z',
      },
    ],
  },
};

export const visibilityPrivate = {
  status: 200,
  body: { public: false },
};

export const noShares = {
  status: 200,
  body: { shares: [], org_shares: [] },
};

export const noOrgs = {
  status: 200,
  body: { orgs: [] },
};

/**
 * Empty preferences — the wire shape is `{theme?, debug_logging?, ...}`
 * all optional, so `{}` represents "no stored prefs, use defaults".
 * Bound into every scenario's base so `getTheme`'s server reconcile
 * (added in the cloud-sync feature) doesn't 599 on no-fixture every
 * authenticated render.
 */
export const emptyPreferences = {
  status: 200,
  body: {},
};

/**
 * Default sitewide appearance config — `GET /v1/appearance`. The root
 * layout fetches this unconditionally (signed-in AND signed-out) to
 * resolve the theme-switch wave speed, so per the fixture-default rule
 * it needs a base entry or every scenario 599s on layout render.
 */
export const defaultAppearance = {
  status: 200,
  body: { theme_wave_speed: 'normal' },
};

/**
 * Admin SMTP config — `GET /v1/admin/smtp`.
 *
 * `/admin/settings` fetches SMTP, appearance and ship-matrix in ONE
 * render, so all three need base entries or the consolidated page 599s
 * on a missing fixture. `password_set` is deliberately true: the form
 * renders a "leave blank to keep" affordance off it.
 */
export const defaultSmtpConfig = {
  status: 200,
  body: {
    enabled: true,
    from_addr: 'noreply@example.test',
    from_name: 'StarStats',
    host: 'smtp.example.test',
    password_set: true,
    port: 587,
    secure: true,
    username: 'starstats',
    web_origin: 'https://example.test',
  },
};

/** Admin Ship Matrix config — `GET /v1/admin/ship-matrix`. */
export const defaultShipMatrixConfig = {
  status: 200,
  body: { media_enabled: true },
};

/** Admin appearance config — `GET /v1/admin/appearance`. */
export const defaultAdminAppearance = {
  status: 200,
  body: { theme_wave_speed: 'normal' },
};

/**
 * Empty category stats — `GET /v1/reference/{category}/stats`. Bound
 * into every scenario's base map so KB detail page renders (which call
 * `getCategoryStats` at layout/detail time) don't 599 on missing
 * fixture. Tests that care about stats content override per-category.
 */
export const emptyCategoryStats = {
  status: 200,
  body: { groups: {} as Record<string, unknown> },
};

/**
 * Empty compare result — `GET /v1/reference/{category}/compare`. Bound
 * into every scenario's base map so KB detail page renders (which call
 * the compare endpoint when a user adds a ship) don't 599 on missing
 * fixture. Real compare requests carry `?slugs=…` but the mock
 * dispatcher matches path-only (strips query), so the path-only key
 * covers all slug combinations. No existing e2e scenario triggers a
 * compare fetch (user interaction required), so this is belt-and-
 * suspenders — matches the same pattern as the `/stats` entries above.
 */
export const emptyCompare = {
  status: 200,
  body: { entries: [] as Array<unknown> },
};

/**
 * Default cohort-members fixture — `GET /v1/reference/{category}/cohort`.
 * Like `/compare`, only fired by user interaction (the tray "Add cohort"
 * picker), so belt-and-suspenders per the layout-level fixture rule.
 */
export const emptyCohort = {
  status: 200,
  body: { entries: [] as Array<unknown> },
};

export const ownedOrgs = {
  status: 200,
  body: {
    orgs: [
      {
        id: 'org_1',
        name: 'Test Squadron',
        slug: 'test-squadron',
        owner_user_id: 'user_existing',
        created_at: '2026-01-01T00:00:00Z',
      },
      {
        id: 'org_2',
        name: 'Aegis Pilots',
        slug: 'aegis-pilots',
        owner_user_id: 'user_existing',
        created_at: '2026-02-01T00:00:00Z',
      },
    ],
  },
};

export function orgDetail(slug: string, name: string) {
  return {
    status: 200,
    body: {
      org: {
        id: `org_${slug}`,
        name,
        slug,
        owner_user_id: 'user_existing',
        created_at: '2026-01-01T00:00:00Z',
      },
      members: [
        { handle: 'TestPilot', role: 'owner' },
        { handle: 'WingmanOne', role: 'admin' },
      ],
      your_role: 'owner',
    },
  };
}

export const publicSummaryShared = {
  status: 200,
  body: {
    claimed_handle: 'JohnSomeone',
    total: 42,
    by_type: [
      { event_type: 'login', count: 30 },
      { event_type: 'death', count: 12 },
    ],
  },
};

/** Stock public summary for `TestPilot`, the suite's default handle. */
export const publicSummaryTestPilot = {
  status: 200,
  body: {
    claimed_handle: 'TestPilot',
    total: 42,
    by_type: [
      { event_type: 'login', count: 30 },
      { event_type: 'death', count: 12 },
    ],
  },
};

export const notFound = {
  status: 404,
  body: { error: 'not_found' },
};

/**
 * Default-empty KB listing — `GET /v1/reference/{category}`. Bound
 * into every scenario's base map so dashboard + journey renders
 * that call `loadAllReferenceBundles()` don't 599 on missing
 * fixture. Tests that care about KB content (e.g. /kb/* browse
 * flows) override per-category with `kbListing(...)` below.
 */
/** Empty KB listing — used in the scenarioFor base map so default
 *  scenarios don't 599 on the dashboard / journey layer fetching
 *  reference bundles. Empty `entries` means there's no summary to
 *  validate; the discriminator-required-on-each-entry rule only
 *  applies when entries are present. */
export const emptyReferenceListing = {
  status: 200,
  body: { entries: [] as Array<unknown> },
};

/** Discriminated summary union — mirrors the server's `Summary`
 *  enum so fixture-builders below stay in sync with the production
 *  contract. Each variant must include the `category` tag because
 *  the production wire shape is internally tagged. */
export type KbSummary =
  | { category: 'vehicle'; manufacturer?: string; role?: string; hull_size?: string; focus?: string }
  | { category: 'weapon'; manufacturer?: string; size?: string; damage_type?: string; weapon_type?: string }
  | { category: 'item'; manufacturer?: string; item_type?: string; grade?: string }
  | {
      category: 'location';
      system?: string;
      parent?: string;
      tag?: string;
      classification?: string;
    };

/** Fixture-builder for a populated KB listing — used by /kb/*
 *  scenario overrides. Entries are stripped of the full metadata
 *  blob to match the production `ReferenceListEntry` shape. The
 *  `category` arg is required so the summary's discriminator can
 *  default correctly when callers don't supply one explicitly. */
export function kbListing(
  category: 'vehicle' | 'weapon' | 'item' | 'location',
  entries: Array<{
    class_name: string;
    display_name: string;
    slug?: string;
    summary?: Omit<KbSummary, 'category'>;
  }>,
) {
  return {
    status: 200,
    body: {
      entries: entries.map((e) => ({
        class_name: e.class_name,
        display_name: e.display_name,
        slug: e.slug ?? null,
        summary: { category, ...(e.summary ?? {}) },
      })),
    },
  };
}

/** Fixture-builder for a single KB detail — used by /kb/{cat}/{slug}
 *  scenarios. Includes full `metadata` since the detail endpoint
 *  returns the unstripped entry. */
export function kbDetail(entry: {
  category: 'vehicle' | 'weapon' | 'item' | 'location';
  class_name: string;
  display_name: string;
  slug: string;
  summary?: Omit<KbSummary, 'category'>;
  metadata?: Record<string, unknown>;
}) {
  return {
    status: 200,
    body: {
      category: entry.category,
      class_name: entry.class_name,
      display_name: entry.display_name,
      slug: entry.slug,
      summary: { category: entry.category, ...(entry.summary ?? {}) },
      metadata: entry.metadata ?? {},
    },
  };
}

/**
 * Default playtime stats — `GET /v1/me/stats/playtime`. Bound into
 * scenarioFor's base map: the `/me` page (a widely-rendered authed
 * surface, reachable via the LeftRail "Me" entry) fetches this on
 * every render, so per the docs/ENGINEERING.md Playwright fixture-default rule it
 * needs a base fixture or any scenario that lands on `/me` 599s with
 * no_mock_fixture. Mirrors `PlaytimeStatsResponse`.
 */
export const playtimeStats = {
  status: 200,
  body: {
    hours: 720,
    session_count: 12,
    total_playtime_secs: 418 * 3600,
  },
};

/**
 * Default locations-visited stats — `GET /v1/me/stats/locations`. Same
 * fixture-default rationale as `playtimeStats` above. Mirrors
 * `LocationsStatsResponse` (the `/me` header only reads
 * `unique_locations`, but the full shape keeps the fixture honest).
 */
export const locationsStats = {
  status: 200,
  body: {
    hours: 720,
    unique_locations: 67,
    top_locations: [
      { value: 'Stanton|Crusader|Orison', count: 30 },
      { value: 'Stanton|microTech|New Babbage', count: 22 },
    ],
  },
};

/**
 * Default combat stats — `GET /v1/me/stats/combat`. Bound into
 * scenarioFor's base map: the `/me` page now fetches this for the
 * identity header K/D row, so per the docs/ENGINEERING.md Playwright
 * fixture-default rule it needs a base entry or any scenario that
 * renders `/me` 599s with no_mock_fixture. Mirrors
 * `CombatStatsResponse`.
 */
export const combatStats = {
  status: 200,
  body: {
    hours: 720,
    kills: 0,
    deaths: 0,
    top_weapons: [],
    deaths_by_zone: [],
  },
};

/**
 * Default Player Facts — `GET /v1/me/facts` (#368). Bound into scenarioFor's
 * base map: the `facts` widget ships ENABLED in `HOME_DEFAULT_LAYOUT`, so
 * every scenario that renders `/me` now fetches this. Per the docs/ENGINEERING.md
 * Playwright fixture-default rule it needs a base entry or those scenarios
 * 599 with no_mock_fixture.
 *
 * Defaults to the "too new" state so the fixture asserts nothing about
 * specific claims — a scenario that cares supplies its own.
 */
export const noPlayerFacts = {
  status: 200,
  body: {
    facts: [],
    enough_history: false,
    sessions_considered: 0,
    sessions_required: 8,
  },
};

/**
 * Default RSI profile snapshot — `GET /v1/me/profile`. Bound into
 * scenarioFor's base map: the `/me` page (Plan 4 redirect target)
 * fetches this on every render, so per the docs/ENGINEERING.md Playwright
 * fixture-default rule it needs a base entry or any scenario that
 * lands on `/me` 599s with no_mock_fixture. Mirrors
 * `ProfileResponse` (all optional fields set to null/empty).
 */
export const emptyProfile = {
  status: 200,
  body: {
    badges: [] as Array<{ name: string; image_url?: string | null }>,
    bio: null,
    captured_at: '2026-01-01T00:00:00Z',
    display_name: null,
    enlistment_date: null,
    location: null,
    primary_org_summary: null,
  },
};

export const unauthorized = {
  status: 401,
  body: { error: 'invalid_credentials' },
};

/**
 * Empty reference resolve response — `POST /v1/reference/resolve`. Bound
 * into every scenario's base map: the `/me` loadout widget (and the
 * `/me/loadout` page) call this endpoint to resolve item class names to
 * friendly display names. Without a base fixture any scenario that renders
 * the loadout widget 599s with no_mock_fixture. Tests that exercise
 * resolved names override this with richer values.
 */
export const emptyResolve = {
  status: 200,
  body: {
    resolved: {} as Record<string, unknown>,
  },
};

export const conflict = (code: string) => ({
  status: 409,
  body: { error: code },
});

/**
 * Two published parser rules — one enabled, one retracted — for
 * `GET /v1/admin/parser-rules` (Task 7). Page-level fixture: only
 * `/admin/parser-rules` calls this endpoint, so it is NOT bound into
 * `scenarioFor`'s base map; tests opt in via a scenario override.
 * Mirrors `AdminParserRulesListResponse` / `AdminParserRuleRow`.
 */
export const adminParserRulesListing = {
  status: 200,
  body: {
    rules: [
      {
        rule_id: 'combat.kill_v1',
        event_name: 'combat_kill',
        match_kind: 'event_name',
        body_regex: '',
        fields: ['actor', 'victim'],
        enabled: true,
      },
      {
        rule_id: 'travel.jump_v1',
        event_name: 'travel_jump',
        match_kind: 'event_name',
        body_regex: '',
        fields: [] as string[],
        enabled: false,
      },
    ],
  },
};

/**
 * `POST /v1/admin/parser-rules` fixture-builder — used by both the
 * parser-rules management page (retract/enable toggle) and the
 * parser-submissions detail page's "Publish rule" panel. Mirrors
 * `PublishRuleResponse`.
 */
export function publishRuleResponse(ruleId: string, enabled: boolean) {
  return {
    status: 200,
    body: { rule_id: ruleId, enabled },
  };
}

/**
 * Known event-type keys for `GET /v1/admin/event-types` (Task 6/7/8).
 * Page-level fixture: only the inference-rule authoring form and the
 * management list's row summaries depend on this — it is NOT bound
 * into `scenarioFor`'s base map. Mirrors `EventTypesResponse`.
 */
export const adminEventTypes = {
  status: 200,
  body: {
    event_types: [
      'vehicle_destruction',
      'resolve_spawn',
      'player_death',
      'travel_jump',
      'npc_kill',
    ],
  },
};

/**
 * Two published inference rules — one enabled, one retracted — for
 * `GET /v1/admin/parser-inference-rules` (Task 8, mirrors Task 7's
 * `adminParserRulesListing`). Page-level fixture: only
 * `/admin/parser-inference-rules` calls this endpoint, so it is NOT
 * bound into `scenarioFor`'s base map. Mirrors
 * `AdminInferenceRulesListResponse` / `AdminInferenceRuleRow` — each
 * row carries a full nested `InferenceRuleDto` (trigger/followups/emits).
 */
export const adminInferenceRulesListing = {
  status: 200,
  body: {
    rules: [
      {
        rule_id: 'combat.kill_streak_v1',
        enabled: true,
        definition: {
          id: 'combat.kill_streak_v1',
          confidence: 0.75,
          window_secs: 30,
          trigger: {
            event_type: 'vehicle_destruction',
            field_equals: { cause: 'combat' },
          },
          followups: [
            { event_type: 'player_death', field_equals: {} as Record<string, string> },
          ],
          emits: {
            event_type: 'resolve_spawn',
            fields: { actor: '${trigger.actor}' },
          },
        },
      },
      {
        rule_id: 'travel.jump_chain_v1',
        enabled: false,
        definition: {
          id: 'travel.jump_chain_v1',
          confidence: 0.6,
          window_secs: 15,
          trigger: {
            event_type: 'travel_jump',
            field_equals: {} as Record<string, string>,
          },
          followups: [] as Array<{
            event_type: string;
            field_equals: Record<string, string>;
          }>,
          emits: {
            event_type: 'travel_jump',
            fields: {} as Record<string, string>,
          },
        },
      },
    ],
  },
};

/**
 * `POST /v1/admin/parser-inference-rules` fixture-builder — used by
 * both the authoring form (Task 6) and the management list's
 * retract/enable toggle (Task 7). Mirrors `PublishInferenceRuleResponse`.
 */
export function publishInferenceRuleResponse(ruleId: string, enabled: boolean) {
  return {
    status: 200,
    body: { rule_id: ruleId, enabled },
  };
}

/**
 * Compose a scenario from the most common "logged in user with data
 * everywhere" defaults plus per-test overrides. Override keys take
 * precedence over the defaults.
 */
/**
 * Default dwell breakdown — `GET /v1/me/location/breakdown`. Deliberately
 * UNEVEN, and deliberately disagreeing with visit order: Lorville is visited
 * once but dwelt in longest, so a test that ranks by visits and one that ranks
 * by dwell cannot both pass on the same ordering.
 */
export const defaultLocationBreakdown = {
  status: 200,
  body: {
    hours: 168,
    entries: [
      { system: 'Stanton', planet: 'Crusader', city: 'Orison', dwell_seconds: 5400, visit_count: 2 },
      { system: 'Stanton', planet: 'ArcCorp', city: 'Area18', dwell_seconds: 1800, visit_count: 1 },
      { system: 'Stanton', planet: 'Hurston', city: 'Lorville', dwell_seconds: 9000, visit_count: 1 },
    ],
  },
};

/** Default current-location — `GET /v1/me/location/current`. The TopBar
 * (signed-in layout) fetches it on EVERY render (H4), so per the
 * fixture-default rule it needs a base entry or every scenario logs a
 * `599 no_mock_fixture`. `location: null` = no current location → the
 * chip renders nothing. Tests that exercise the chip override this. */
const noCurrentLocation = { status: 200, body: { location: null } };

/** Default inbound shares — `GET /v1/me/shared-with-me`. Same layout-level
 * reason (H4): the AccountMenu inbound badge reads it on every signed-in
 * render. Field is `shared_with_me` (not `entries`). Empty = no badge. */
const noSharedWithMe = { status: 200, body: { shared_with_me: [] } };

/** Default hangar snapshot — `GET /v1/me/hangar`. The `/me` hangar widget
 * fetches it on render; `getMyHangar` maps 404 → null so the widget shows
 * nothing. Base 404 keeps scenarios that render `/me` from a 599 (H4). */
const noHangarSnapshot = { status: 404, body: { error: 'not_found' } };

/** No stored layout — the surface falls back to its curated default. */
export const projectionLayoutDefault: MockResponse = {
  status: 200,
  body: { layout: null },
};

/** No location telemetry: the ring draws no map and says so. */
export const locationTraceEmpty: MockResponse = {
  status: 200,
  body: { entries: [] },
};


/**
 * Tray release feed — the GitHub Releases shape, served from the mock server
 * because `playwright.config.ts` points `STARSTATS_RELEASES_API` at it.
 *
 * Bound into `scenarioFor`'s base map: `/downloads` (the Emitter) absorbed
 * `/devices`, so the auth flow and the pairing captures all render this page.
 * Per the fixture-default rule, a widely-rendered fetch without a base entry
 * makes every scenario that touches the surface fail.
 *
 * Deliberately a `tray-v` tag with one Windows and one Linux asset and NO
 * macOS build — that is the real state of the track, and it keeps the
 * "No macOS build yet" branch on screen where it can be seen.
 */
export const trayReleases = {
  status: 200,
  body: [
    {
      tag_name: 'tray-v1.8.31',
      name: 'Tray v1.8.31',
      body: 'Loadout capture fixes.',
      draft: false,
      prerelease: false,
      published_at: '2026-08-01T12:00:00Z',
      html_url: 'https://github.com/TheCodeSaiyan/StarStats/releases/tag/tray-v1.8.31',
      assets: [
        {
          name: 'StarStats_1.8.31_x64-setup.exe',
          browser_download_url: 'https://example.invalid/StarStats_1.8.31_x64-setup.exe',
          size: 8_912_896,
        },
        {
          name: 'StarStats_1.8.31_amd64.AppImage',
          browser_download_url: 'https://example.invalid/StarStats_1.8.31_amd64.AppImage',
          size: 10_485_760,
        },
      ],
    },
    {
      tag_name: 'tray-v1.9.0-beta.1',
      name: 'Tray v1.9.0-beta.1',
      body: 'Next line.',
      draft: false,
      prerelease: true,
      published_at: '2026-08-14T12:00:00Z',
      html_url: 'https://github.com/TheCodeSaiyan/StarStats/releases/tag/tray-v1.9.0-beta.1',
      assets: [
        {
          name: 'StarStats_1.9.0-beta.1_x64-setup.exe',
          browser_download_url: 'https://example.invalid/StarStats_1.9.0-beta.1_x64-setup.exe',
          size: 8_950_000,
        },
      ],
    },
  ],
};

/**
 * Default per-scope sharing — `GET /v1/users/me/share-scopes` and the public
 * per-handle mirror. Three published, two withheld, so a test that renders a
 * profile exercises BOTH halves of the published/withheld statement rather
 * than only the branch that happens to be non-empty.
 */
export const defaultShareScopes = {
  status: 200,
  body: {
    combat_mission: true,
    economy: false,
    travel: true,
    records: false,
    recent_activity: true,
  },
};

export function scenarioFor(
  id: string,
  overrides: ScenarioRoutes = {},
): Scenario {
  const base: ScenarioRoutes = {
    'GET /v1/auth/me': currentUser,
    'GET /v1/me/summary': summaryWithEvents,
    'GET /v1/me/events': eventsPageDescending,
    'GET /v1/me/timeline': timeline30Days,
    'GET /v1/auth/devices': deviceList,
    'GET /gh/releases': trayReleases,
    'GET /v1/me/visibility': visibilityPrivate,
    'GET /v1/me/shares': noShares,
    'GET /v1/me/preferences': emptyPreferences,
    'GET /v1/appearance': defaultAppearance,
    // /admin/settings fetches these three in a single render.
    'GET /v1/admin/smtp': defaultSmtpConfig,
    'GET /v1/admin/appearance': defaultAdminAppearance,
    'GET /v1/admin/ship-matrix': defaultShipMatrixConfig,
    // The root layout and auth pages read this public flag. Gate-off is
    // the dormant default; beta-specific tests override it explicitly.
    'GET /v1/waitlist/status': { status: 200, body: { gate_enabled: false } },
    'GET /v1/orgs': noOrgs,
    // Per-scope sharing. `/u/[handle]` reads these to state what a pilot
    // publishes and what they withhold — the owner's via `/v1/users/me`, a
    // visitor's via the unauthenticated public route. Without a default, the
    // page cannot tell "publishes nothing" from "the endpoint did not answer"
    // and correctly renders the latter, so every profile scenario would assert
    // against the failure state.
    //
    // KEYED PER HANDLE because the mock's wildcards are PREFIX-only: a
    // `GET /v1/public/*` entry would also swallow `/summary`, `/profile` and
    // `/timeline` for every scenario that has no exact key of its own.
    // `/u/TestPilot` is the stock profile every scenario reaches for. Without
    // a summary it resolves to "not available" and a test asserting on the
    // profile body waits out its timeout against the refused view.
    'GET /v1/public/TestPilot/summary': publicSummaryTestPilot,
    'GET /v1/users/me/share-scopes': defaultShareScopes,
    'GET /v1/public/TestPilot/share-scopes': defaultShareScopes,
    'GET /v1/public/JohnSomeone/share-scopes': defaultShareScopes,
    // KB v1: dashboard + journey now call `loadAllReferenceBundles()`
    // which fans out to all four categories. Default each to an
    // empty listing so scenarios that don't care about KB content
    // still render without 599s. Tests that exercise /kb/* override
    // these via `kbListing(...)` per scenario.
    'GET /v1/reference/vehicle': emptyReferenceListing,
    'GET /v1/reference/weapon': emptyReferenceListing,
    'GET /v1/reference/item': emptyReferenceListing,
    'GET /v1/reference/location': emptyReferenceListing,
    // KB detail page calls `getCategoryStats` for each category at
    // layout/detail render time. Default each to empty groups so
    // existing scenarios don't 599 with no_mock_fixture.
    'GET /v1/reference/vehicle/stats': emptyCategoryStats,
    'GET /v1/reference/weapon/stats': emptyCategoryStats,
    'GET /v1/reference/item/stats': emptyCategoryStats,
    'GET /v1/reference/location/stats': emptyCategoryStats,
    // KB detail compare endpoint — `GET /v1/reference/{category}/compare`.
    // Only fires on user interaction (adding a ship), so no current
    // scenario triggers it; fixture is belt-and-suspenders so any future
    // layout-level call doesn't 599 with no_mock_fixture.
    'GET /v1/reference/vehicle/compare': emptyCompare,
    'GET /v1/reference/weapon/compare': emptyCompare,
    'GET /v1/reference/item/compare': emptyCompare,
    'GET /v1/reference/location/compare': emptyCompare,
    // KB cohort bulk-add endpoint — `GET /v1/reference/{category}/cohort`.
    'GET /v1/reference/vehicle/cohort': emptyCohort,
    'GET /v1/reference/weapon/cohort': emptyCohort,
    'GET /v1/reference/item/cohort': emptyCohort,
    'GET /v1/reference/location/cohort': emptyCohort,
    // Public roadmap surface (Phase 5). The /roadmap and /changelog
    // pages call these; default each to an empty list so scenarios
    // that hit those routes render without 599s.
    'GET /v1/roadmap': emptyRoadmapListing,
    'GET /v1/roadmap/changelog': emptyChangelog,
    // Supporter chip in TopBar fans out to /v1/me/supporter on every
    // signed-in render. Default to state=none so scenarios that
    // don't exercise supporter logic don't hit a 599; supporter
    // tests override via `supporterStatus(...)`.
    'GET /v1/me/supporter': supporterStatusNone,
    // TopBar (signed-in layout) fans out to the current-location chip and
    // the inbound-share badge on every render. Both are load-bearing
    // layout fetches, so per the fixture-default rule they get base
    // entries — without them every signed-in scenario emits a
    // `599 no_mock_fixture` for these two paths (H4).
    'GET /v1/me/location/current': noCurrentLocation,
    // Per-place dwell. `/me/travel` ranks its taxonomy by this and falls back
    // to visit counts when it is absent — without a default, every scenario
    // would exercise only the fallback and the dwell path would ship untested.
    'GET /v1/me/location/breakdown': defaultLocationBreakdown,
    'GET /v1/me/shared-with-me': noSharedWithMe,
    // The `/me` hangar widget fetches this on render; 404 = no snapshot
    // (server holds no RSI creds) → the widget shows nothing, no 599.
    'GET /v1/me/hangar': noHangarSnapshot,
    // The `/me` home page fetches playtime + locations + combat stats
    // for its identity header on every render. Reachable from the
    // LeftRail "Me" entry, so per the fixture-default rule all three
    // get a base entry; /me-specific tests override with richer values.
    'GET /v1/me/facts': noPlayerFacts,
    'GET /v1/me/stats/playtime': playtimeStats,
    'GET /v1/me/stats/locations': locationsStats,
    'GET /v1/me/stats/combat': combatStats,
    // The `/me` page (Plan 4 redirect target) also fetches the RSI
    // profile snapshot. Default to an empty-nullable stub so any
    // scenario that redirects to /me doesn't 599 with no_mock_fixture.
    // Tests that exercise the profile card override with richer values.
    'GET /v1/me/profile': emptyProfile,
    // The `/me` loadout widget (and `/me/loadout` page) call this to
    // resolve item class names to friendly display names. Per the
    // Playwright fixture-default rule: any scenario that renders the
    // loadout widget would 599 without this base entry. Tests that
    // exercise resolved names override with richer values.
    'POST /v1/reference/resolve': emptyResolve,
    // The `/me` projection reads the reader's saved element layout and the
    // location trace that feeds the ring's map mode. Both degrade quietly on
    // failure (default layout / empty ring), but per the fixture-default rule
    // they get base entries so a scenario exercises the real render path
    // rather than the degraded one.
    'GET /v1/users/me/profile-layout': projectionLayoutDefault,
    'GET /v1/me/location/trace': locationTraceEmpty,
  };
  return { __id: id, routes: { ...base, ...overrides } };
}

/** Empty public roadmap list — used in scenarioFor's base map. */
export const emptyRoadmapListing = {
  status: 200,
  body: { items: [] as Array<unknown> },
};

/** Empty published changelog — used in scenarioFor's base map. */
export const emptyChangelog = {
  status: 200,
  body: { entries: [] as Array<unknown> },
};

/** Non-supporter status — used in scenarioFor's base map. */
export const supporterStatusNone = {
  status: 200,
  body: {
    state: 'none',
    name_plate: null,
    became_supporter_at: null,
    last_payment_at: null,
    grace_until: null,
    cancelled_at: null,
    current_tier_key: null,
  },
};

/** Helper for tests that want an active supporter state with a tier. */
export const supporterStatus = (
  tier: 'coffee' | 'standard' | 'generous',
  namePlate: string | null = null,
) => ({
  status: 200,
  body: {
    state: 'active',
    name_plate: namePlate,
    became_supporter_at: '2026-05-31T22:00:00Z',
    last_payment_at: '2026-05-31T22:00:00Z',
    grace_until: '2026-06-30T22:00:00Z',
    cancelled_at: null,
    current_tier_key: tier,
  },
});
