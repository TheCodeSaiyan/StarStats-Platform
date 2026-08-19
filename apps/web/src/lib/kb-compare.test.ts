import { describe, it, expect, vi, afterEach } from 'vitest';
import { fetchCompareVectors } from './kb-compare';

afterEach(() => { vi.unstubAllGlobals(); });

describe('fetchCompareVectors', () => {
  it('requests the same-origin route with joined slugs and returns entries', async () => {
    const calls: string[] = [];
    vi.stubGlobal('fetch', vi.fn(async (url: string) => {
      calls.push(url);
      return { ok: true, json: async () => ({ entries: [{ slug: 'a', display_name: 'A', class_name: 'A', peer_group: 'combat', metrics: {} }] }) } as Response;
    }));
    const out = await fetchCompareVectors('vehicle', ['a', 'b']);
    expect(calls[0]).toBe('/kb/compare/vehicle?slugs=a%2Cb');
    expect(out.entries).toHaveLength(1);
  });

  it('returns empty entries on a non-ok response', async () => {
    vi.stubGlobal('fetch', vi.fn(async () => ({ ok: false, status: 400, json: async () => ({}) }) as Response));
    const out = await fetchCompareVectors('vehicle', ['x']);
    expect(out.entries).toEqual([]);
  });

  it('short-circuits to empty for an empty slug list', async () => {
    const f = vi.fn();
    vi.stubGlobal('fetch', f);
    const out = await fetchCompareVectors('vehicle', []);
    expect(out.entries).toEqual([]);
    expect(f).not.toHaveBeenCalled();
  });
});
