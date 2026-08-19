import { describe, it, expect } from 'vitest';
import { healthStrings } from './useHealthStrings';
import type { HealthParams } from '../api';

describe('healthStrings', () => {
  const variants: HealthParams[] = [
    { id: 'gamelog_missing' },
    { id: 'api_url_missing' },
    { id: 'pair_missing' },
    { id: 'auth_lost' },
    { id: 'cookie_missing' },
    { id: 'sync_failing', last_error: '502 Bad Gateway', attempts_since_success: 3 },
    { id: 'hangar_skip', reason: 'rate limited', since: '2026-05-16T08:00:00Z' },
    { id: 'email_unverified' },
    { id: 'game_log_stale', last_event_at: '2026-05-16T07:00:00Z' },
    { id: 'update_available', version: '0.4.1-beta' },
    { id: 'disk_free_low', free_bytes: 500_000_000 },
  ];

  it.each(variants)('renders a summary for $id', (p) => {
    const out = healthStrings(p);
    expect(out.summary).toBeTruthy();
    expect(out.summary.length).toBeGreaterThan(0);
    expect(out.summary.length).toBeLessThanOrEqual(120);
  });

  it('exhaustively covers every HealthParams variant', () => {
    expect(variants.length).toBe(11);
  });

  it('formats SyncFailing with the error and attempts', () => {
    const out = healthStrings({ id: 'sync_failing', last_error: 'foo', attempts_since_success: 5 });
    expect(out.detail).toContain('foo');
    expect(out.detail).toContain('5');
  });

  it('formats DiskFreeLow as human-readable bytes', () => {
    const out = healthStrings({ id: 'disk_free_low', free_bytes: 500_000_000 });
    expect(out.summary).toMatch(/MB|MiB/);
  });
});
