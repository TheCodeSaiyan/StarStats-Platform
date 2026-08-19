import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';

// Mock the server action so the client hook module's import graph stays hermetic.
vi.mock('@/app/kb/actions', () => ({ saveKbPrefs: vi.fn() }));

import { resolveInitialKbPrefs, useKbPrefs, KB_PREFS_STORAGE_KEY } from './kb-prefs';

describe('resolveInitialKbPrefs', () => {
  beforeEach(() => {
    const store: Record<string, string> = {};
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => store[k] ?? null,
      setItem: (k: string, v: string) => { store[k] = v; },
      removeItem: (k: string) => { delete store[k]; },
    });
  });

  it('prefers server prefs when signed in', () => {
    const p = resolveInitialKbPrefs({ view: 'compact', units: 'imperial' });
    expect(p).toEqual({ view: 'compact', units: 'imperial' });
  });

  it('reads localStorage when no server prefs (signed out)', () => {
    localStorage.setItem(KB_PREFS_STORAGE_KEY, JSON.stringify({ view: 'compact', units: 'metric' }));
    const p = resolveInitialKbPrefs(null);
    expect(p.view).toBe('compact');
  });

  it('defaults to visual/metric when nothing stored', () => {
    const p = resolveInitialKbPrefs(null);
    expect(p).toEqual({ view: 'visual', units: 'metric' });
  });

  it('ignores malformed localStorage', () => {
    localStorage.setItem(KB_PREFS_STORAGE_KEY, '{not json');
    expect(resolveInitialKbPrefs(null).view).toBe('visual');
  });

  it('ignores undefined server fields, keeping defaults', () => {
    expect(resolveInitialKbPrefs({ view: 'compact', units: undefined })).toEqual({ view: 'compact', units: 'metric' });
  });

  it('merges server over localStorage per field', () => {
    localStorage.setItem(KB_PREFS_STORAGE_KEY, JSON.stringify({ view: 'visual', units: 'imperial' }));
    expect(resolveInitialKbPrefs({ view: 'compact' })).toEqual({ view: 'compact', units: 'imperial' });
  });
});

describe('useKbPrefs', () => {
  beforeEach(() => {
    const store: Record<string, string> = {};
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => store[k] ?? null,
      setItem: (k: string, v: string) => { store[k] = v; },
      removeItem: (k: string) => { delete store[k]; },
    });
  });

  it('initial state ignores localStorage (SSR-safe) then reconciles after mount', async () => {
    // Spy on the initializer's storage access. The SSR-safe init must NOT
    // touch localStorage; only the post-mount effect (via resolveInitialKbPrefs)
    // may read it. renderHook flushes effects under act() before returning, so
    // result.current already reflects the reconciled value — we assert the
    // *initializer* stayed storage-free by snapshotting getItem calls up to the
    // first render, then confirm the effect applied the stored pref.
    localStorage.setItem(KB_PREFS_STORAGE_KEY, JSON.stringify({ view: 'compact' }));
    const getItem = vi.spyOn(localStorage, 'getItem');
    getItem.mockClear();

    const { result } = renderHook(() => useKbPrefs({ serverPrefs: null, signedIn: false }));

    // The effect (which calls resolveInitialKbPrefs → readLocal) is the ONLY
    // path that reads storage; if the initializer had read it too, getItem
    // would have fired more than once. resolveInitialKbPrefs reads once.
    expect(getItem).toHaveBeenCalledTimes(1);
    // after mount effect, localStorage applied
    await waitFor(() => expect(result.current.prefs.view).toBe('compact'));
  });
});
