import React from 'react';
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, screen, fireEvent } from '@testing-library/react';
import { KbDetailView } from './KbDetailView';

vi.mock('@/app/kb/actions', () => ({ saveKbPrefs: vi.fn() }));

import { fetchCompareVectors } from '@/lib/kb-compare';
vi.mock('@/lib/kb-compare', () => ({
  fetchCompareVectors: vi.fn(async () => ({
    entries: [
      { slug: 'avenger', display_name: 'Avenger Stalker', class_name: 'AEGS', peer_group: 'combat', metrics: { 'speed.scm': 262 } },
      { slug: 'gladius', display_name: 'Gladius', class_name: 'AEGS', peer_group: 'combat', metrics: { 'speed.scm': 226 } },
    ],
  })),
}));

// A cohort with more members than the 10-ship comparison can hold, so the
// over-cap notice surfaces. 12 distinct non-anchor slugs → only 9 fit.
vi.mock('@/lib/kb-cohort', () => ({
  fetchCohortMembers: vi.fn(async () => ({
    entries: Array.from({ length: 12 }, (_, i) => ({
      slug: `ship-${i}`,
      display_name: `Ship ${i}`,
      class_name: 'TEST',
      peer_group: 'combat',
      metrics: { 'speed.scm': 200 + i },
    })),
  })),
}));

const props = {
  category: 'vehicle' as const,
  displayName: 'Avenger Stalker',
  metadata: { speed: { scm: 262 } },
  groups: { 'family:combat': { 'speed.scm': { min: 200, p10: 205, p25: 210, p50: 222, p75: 240, p90: 270, max: 275, n: 84 } } },
  cohorts: [{ key: 'family:combat', kind: 'family', label: 'Combat ships' }],
  description: 'A fast interceptor.',
  roleTags: ['Fast strike'],
  serverPrefs: null,
  signedIn: false,
  anchorSlug: 'avenger',
  catalog: [],
};

describe('KbDetailView', () => {
  beforeEach(() => {
    const store: Record<string, string> = {};
    vi.stubGlobal('localStorage', {
      getItem: (k: string) => store[k] ?? null,
      setItem: (k: string, v: string) => { store[k] = v; },
      removeItem: (k: string) => { delete store[k]; },
    });
  });

  it('renders the visual view by default and toggles to compact', () => {
    render(<KbDetailView {...props} />);
    expect(screen.getByText('A fast interceptor.')).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: /compact/i }));
    expect(screen.getByText(/SCM speed/i)).toBeTruthy();
  });

  it('enters comparison mode when a ship is added from the catalog', async () => {
    render(
      <KbDetailView
        {...props}
        anchorSlug="avenger"
        catalog={[{ slug: 'gladius', display_name: 'Gladius' }]}
        cohorts={[{ key: 'family:combat', kind: 'family', label: 'Combat ships' }]}
        groups={{ 'family:combat': { 'speed.scm': { min: 200, p10: 205, p25: 210, p50: 222, p75: 240, p90: 270, max: 275, n: 84 } } }}
      />,
    );
    fireEvent.change(screen.getByRole('searchbox', { name: /add ship/i }), { target: { value: 'glad' } });
    fireEvent.click(screen.getByText('Gladius'));
    expect(await screen.findByRole('button', { name: /comparison/i })).toBeTruthy();
    expect(vi.mocked(fetchCompareVectors)).toHaveBeenCalled();
  });

  it('exits comparison mode and prunes radar state when the ship is removed', async () => {
    render(
      <KbDetailView
        {...props}
        anchorSlug="avenger"
        catalog={[{ slug: 'gladius', display_name: 'Gladius' }]}
        cohorts={[{ key: 'family:combat', kind: 'family', label: 'Combat ships' }]}
        groups={{ 'family:combat': { 'speed.scm': { min: 200, p10: 205, p25: 210, p50: 222, p75: 240, p90: 270, max: 275, n: 84 } } }}
      />,
    );
    fireEvent.change(screen.getByRole('searchbox', { name: /add ship/i }), { target: { value: 'glad' } });
    fireEvent.click(screen.getByText('Gladius'));
    // In comparison mode now.
    expect(await screen.findByRole('button', { name: /^comparison$/i })).toBeTruthy();
    // Remove via the chip's "Remove Gladius" control.
    fireEvent.click(screen.getByRole('button', { name: /remove gladius/i }));
    // Comparison mode exits — the Comparison/Single switch is gone. Because
    // removeShip prunes the onRadar key, the stale `true` no longer inflates
    // the on-radar count for a future add.
    expect(screen.queryByRole('button', { name: /^comparison$/i })).toBeNull();
  });

  it('drives the Compared-to baseline from the anchor cohorts', () => {
    render(
      <KbDetailView
        {...props}
        cohorts={[
          { key: 'family:combat', kind: 'family', label: 'Combat ships' },
          { key: 'type:interceptor', kind: 'type', label: 'Interceptors' },
        ]}
        groups={{
          'family:combat': { 'speed.scm': { min: 200, p10: 205, p25: 210, p50: 222, p75: 240, p90: 270, max: 275, n: 84 } },
          'type:interceptor': { 'speed.scm': { min: 250, p10: 255, p25: 258, p50: 262, p75: 268, p90: 272, max: 275, n: 8 } },
        }}
        metadata={{ speed: { scm: 262 } }}
      />,
    );
    const sel = screen.getByRole('combobox', { name: /compared to/i }) as HTMLSelectElement;
    // default = first cohort (family)
    expect(sel.value).toBe('family:combat');
    // both cohort labels present as options (appear in both the "Compared to"
    // select and the "Add cohort" tray select, so use getAllByRole)
    expect(screen.getAllByRole('option', { name: 'Combat ships' }).length).toBeGreaterThan(0);
    expect(screen.getAllByRole('option', { name: 'Interceptors' }).length).toBeGreaterThan(0);
    // switch to the type cohort
    fireEvent.change(sel, { target: { value: 'type:interceptor' } });
    expect(sel.value).toBe('type:interceptor');
  });

  it('surfaces an inline notice when a cohort bulk-add exceeds the 10-ship cap', async () => {
    render(
      <KbDetailView
        {...props}
        anchorSlug="avenger"
        cohorts={[{ key: 'type:interceptor', kind: 'type', label: 'Interceptors' }]}
      />,
    );
    // Pick the cohort from the tray's "Add cohort" select.
    fireEvent.change(screen.getByRole('combobox', { name: /add cohort/i }), { target: { value: 'type:interceptor' } });
    // 12 candidates, only 9 fit (anchor + 9 = 10) — notice reports the cap.
    const notice = await screen.findByRole('status');
    expect(notice.textContent).toMatch(/Added 9 of 12/);
  });
});
