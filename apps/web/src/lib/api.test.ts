import { describe, it, expect, vi, beforeEach, beforeAll } from 'vitest';
import {
  setDeviceSync,
  ApiCallError,
  getProfileLayout,
  updateProfileLayout,
  getAdminParserRules,
  publishAdminParserRule,
  getAdminInferenceRules,
  publishAdminInferenceRule,
  publishSubmissionToCommunity,
  getAdminEventTypes,
  getLives,
} from './api';

// `api.ts` reads STARSTATS_API_URL at call time via apiBase().
// Set a fixed origin so URL assertions are predictable.
const API_ORIGIN = 'http://localhost:8080';

beforeAll(() => {
  process.env.STARSTATS_API_URL = API_ORIGIN;
});

const fetchMock = vi.fn();

beforeEach(() => {
  fetchMock.mockReset();
  vi.stubGlobal('fetch', fetchMock);
});

describe('setDeviceSync', () => {
  it('POSTs to /v1/auth/devices/:id/sync with the enabled flag and returns parsed response', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ sync_enabled: true }), { status: 200 }),
    );

    const out = await setDeviceSync('bearer-token', 'device-uuid', true);

    expect(out.sync_enabled).toBe(true);

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit & { headers: Record<string, string> }];

    // URL shape
    expect(String(url)).toMatch(/\/v1\/auth\/devices\/device-uuid\/sync$/);

    // Method
    expect(init.method).toBe('POST');

    // Authorization header — request() sets it as lowercase 'authorization'
    expect(init.headers['authorization']).toBe('Bearer bearer-token');

    // Body
    expect(JSON.parse(init.body as string)).toEqual({ enabled: true });
  });

  it('throws ApiCallError on non-2xx with error detail', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ error: 'device_not_found' }), { status: 404 }),
    );

    await expect(setDeviceSync('t', 'missing', false)).rejects.toThrow(
      /device_not_found/,
    );
  });

  it('throws an instance of ApiCallError with correct status code', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ error: 'device_not_found' }), { status: 404 }),
    );

    await expect(setDeviceSync('t', 'missing', false)).rejects.toBeInstanceOf(
      ApiCallError,
    );
  });

  it('URL-encodes the device id', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ sync_enabled: false }), { status: 200 }),
    );

    await setDeviceSync('t', 'has spaces/and-slashes', false);

    const [url] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(String(url)).toContain('has%20spaces%2Fand-slashes');
  });
});

describe('layout surface query param', () => {
  it('getProfileLayout defaults to the profile surface', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ layout: null, source: 'default' }), { status: 200 }),
    );

    await getProfileLayout('tok');

    const [url] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(String(url)).toContain('/v1/users/me/profile-layout?surface=profile');
  });

  it('getProfileLayout targets the home surface when asked', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ layout: null, source: 'default' }), { status: 200 }),
    );

    await getProfileLayout('tok', 'home');

    const [url] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(String(url)).toContain('/v1/users/me/profile-layout?surface=home');
  });

  it('updateProfileLayout targets the home surface when asked', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ layout: null, source: 'default' }), { status: 200 }),
    );

    await updateProfileLayout('tok', null, 'home');

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(String(url)).toContain('/v1/users/me/profile-layout?surface=home');
    expect(init.method).toBe('PUT');
  });
});

describe('admin parser-rules', () => {
  it('getAdminParserRules GETs /v1/admin/parser-rules', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ rules: [] }), { status: 200 }),
    );

    const res = await getAdminParserRules('tok');

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit & { headers: Record<string, string> }];
    expect(String(url)).toContain('/v1/admin/parser-rules');
    expect(init.method).toBe('GET');
    expect(init.headers['authorization']).toBe('Bearer tok');
    expect(res.rules).toEqual([]);
  });

  it('publishAdminParserRule POSTs to /v1/admin/parser-rules', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(
        JSON.stringify({ rule_id: 'combat.kill.v1', enabled: true }),
        { status: 200 },
      ),
    );

    const res = await publishAdminParserRule('tok', {
      rule_id: 'combat.kill.v1',
      event_name: 'actor_death',
      match_kind: 'event_name',
      body_regex: '',
      fields: [],
      enabled: true,
    });

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit & { headers: Record<string, string> }];
    expect(String(url)).toContain('/v1/admin/parser-rules');
    expect(init.method).toBe('POST');
    expect(init.headers['authorization']).toBe('Bearer tok');
    expect(JSON.parse(init.body as string)).toEqual({
      rule_id: 'combat.kill.v1',
      event_name: 'actor_death',
      match_kind: 'event_name',
      body_regex: '',
      fields: [],
      enabled: true,
    });
    expect(res.rule_id).toBe('combat.kill.v1');
  });
});

describe('publishSubmissionToCommunity', () => {
  it('POSTs label/description/pattern/force_anonymous to the publish endpoint', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          community_submission_id: 'uuid-1',
          already_published: false,
        }),
        { status: 201 },
      ),
    );

    const res = await publishSubmissionToCommunity('tok', 42, {
      proposed_label: 'a.b',
      description: 'd',
      pattern: 'p',
      force_anonymous: true,
    });

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit & { headers: Record<string, string> }];
    expect(String(url)).toContain('/v1/admin/parser-submissions/42/publish');
    expect(init.method).toBe('POST');
    expect(init.headers['authorization']).toBe('Bearer tok');
    expect(JSON.parse(init.body as string)).toEqual({
      proposed_label: 'a.b',
      description: 'd',
      pattern: 'p',
      force_anonymous: true,
    });
    expect(res.community_submission_id).toBe('uuid-1');
    expect(res.already_published).toBe(false);
  });
});

describe('admin inference-rules', () => {
  it('getAdminInferenceRules GETs /v1/admin/parser-inference-rules', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ rules: [] }), { status: 200 }),
    );

    const res = await getAdminInferenceRules('tok');

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit & { headers: Record<string, string> }];
    expect(String(url)).toContain('/v1/admin/parser-inference-rules');
    expect(init.method).toBe('GET');
    expect(init.headers['authorization']).toBe('Bearer tok');
    expect(res.rules).toEqual([]);
  });

  it('publishAdminInferenceRule POSTs to /v1/admin/parser-inference-rules', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(
        JSON.stringify({ rule_id: 'travel.jump.v1', enabled: true }),
        { status: 200 },
      ),
    );

    const body = {
      id: 'travel.jump.v1',
      confidence: 0.9,
      window_secs: 30,
      trigger: { event_type: 'jump_requested', field_equals: {} },
      followups: [{ event_type: 'jump_complete', field_equals: {} }],
      emits: { event_type: 'travel_jump', fields: {} },
      enabled: true,
    };

    const res = await publishAdminInferenceRule('tok', body);

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit & { headers: Record<string, string> }];
    expect(String(url)).toContain('/v1/admin/parser-inference-rules');
    expect(init.method).toBe('POST');
    expect(init.headers['authorization']).toBe('Bearer tok');
    expect(JSON.parse(init.body as string)).toEqual(body);
    expect(res.rule_id).toBe('travel.jump.v1');
  });

  it('getAdminEventTypes GETs /v1/admin/event-types', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ event_types: ['jump_requested'] }), {
        status: 200,
      }),
    );

    const res = await getAdminEventTypes('tok');

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit & { headers: Record<string, string> }];
    expect(String(url)).toContain('/v1/admin/event-types');
    expect(init.method).toBe('GET');
    expect(init.headers['authorization']).toBe('Bearer tok');
    expect(res.event_types).toEqual(['jump_requested']);
  });
});

describe('getLives', () => {
  it('GETs /v1/me/stats/lives and returns the parsed response', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(
        JSON.stringify({
          total_lives: 5,
          deaths: 4,
          mean_life_secs: 600,
          longest_life_secs: 1800,
          sessions: 3,
          deaths_per_session: 1.3,
          lives_ended_by_crash: 1,
          recent_lives: [],
        }),
        { status: 200 },
      ),
    );

    const res = await getLives('tok');

    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit & { headers: Record<string, string> }];
    expect(String(url)).toContain('/v1/me/stats/lives');
    expect(init.method).toBe('GET');
    expect(init.headers['authorization']).toBe('Bearer tok');
    expect(res.total_lives).toBe(5);
    expect(res.deaths_per_session).toBe(1.3);
    expect(res.recent_lives).toEqual([]);
  });
});
