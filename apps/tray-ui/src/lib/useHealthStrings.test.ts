import { describe, it, expect } from 'vitest';
import { healthStrings } from './useHealthStrings';
import type { HealthId, HealthParams } from '../api';

describe('healthStrings', () => {
  // Exhaustive BY CONSTRUCTION: `Record<HealthId, …>` makes TypeScript
  // reject this object the moment a HealthId has no example here, so
  // adding a variant to the union forces a string for it.
  //
  // This replaced an `expect(variants.length).toBe(11)` guard, which
  // only noticed entries being REMOVED from the list — adding
  // `gamelog_override_invalid` to the union left it green.
  const EXAMPLES: Record<HealthId, HealthParams> = {
    gamelog_missing: { id: 'gamelog_missing' },
    gamelog_override_invalid: { id: 'gamelog_override_invalid', path: 'D:\\typo\\Game.log' },
    api_url_missing: { id: 'api_url_missing' },
    pair_missing: { id: 'pair_missing' },
    auth_lost: { id: 'auth_lost' },
    cookie_missing: { id: 'cookie_missing' },
    sync_failing: { id: 'sync_failing', last_error: '502 Bad Gateway', attempts_since_success: 3 },
    hangar_skip: { id: 'hangar_skip', reason: 'rate limited', since: '2026-05-16T08:00:00Z' },
    email_unverified: { id: 'email_unverified' },
    game_log_stale: { id: 'game_log_stale', last_event_at: '2026-05-16T07:00:00Z' },
    update_available: { id: 'update_available', version: '0.4.1-beta' },
    disk_free_low: { id: 'disk_free_low', free_bytes: 500_000_000 },
  };

  const variants: HealthParams[] = Object.values(EXAMPLES);

  it.each(variants)('renders a summary for $id', (p) => {
    const out = healthStrings(p);
    expect(out.summary).toBeTruthy();
    expect(out.summary.length).toBeGreaterThan(0);
    expect(out.summary.length).toBeLessThanOrEqual(120);
  });

  it('keys every example under its own id', () => {
    // Guards the one mistake the Record type cannot catch: an entry
    // filed under the wrong key.
    for (const [id, params] of Object.entries(EXAMPLES)) {
      expect(params.id).toBe(id);
    }
  });

  it('names the offending path when the override is invalid', () => {
    const out = healthStrings({ id: 'gamelog_override_invalid', path: 'D:\\typo\\Game.log' });
    expect(out.detail).toContain('D:\\typo\\Game.log');
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
