import { describe, it, expect, vi, afterEach } from 'vitest';
import { fetchCohortMembers } from './kb-cohort';

afterEach(() => { vi.unstubAllGlobals(); });

describe('fetchCohortMembers', () => {
  it('requests the same-origin route with the encoded key', async () => {
    const calls: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      calls.push(url);
      return { ok: true, json: async () => ({ entries: [{ slug: 'a', display_name: 'A', class_name: 'A', peer_group: 'combat', metrics: {} }] }) } as Response;
    }));
    const out = await fetchCohortMembers('vehicle', 'type:interceptor');
    expect(calls[0]).toBe('/kb/cohort/vehicle?key=type%3Ainterceptor');
    expect(out.entries).toHaveLength(1);
  });

  it('empty entries on non-ok or empty key', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => ({ ok: false }) as Response));
    expect((await fetchCohortMembers('vehicle', 'x:y')).entries).toEqual([]);
    const f = vi.fn();
    vi.stubGlobal('fetch', f);
    expect((await fetchCohortMembers('vehicle', '')).entries).toEqual([]);
    expect(f).not.toHaveBeenCalled();
  });
});
