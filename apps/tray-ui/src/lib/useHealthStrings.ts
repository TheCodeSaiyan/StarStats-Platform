import type { HealthParams } from '../api';

export interface HealthStrings {
  summary: string;
  detail?: string;
}

function fmtBytes(n: number): string {
  if (n >= 1024 * 1024 * 1024) return `${(n / (1024 * 1024 * 1024)).toFixed(1)} GiB`;
  if (n >= 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(0)} MiB`;
  return `${(n / 1024).toFixed(0)} KiB`;
}

function fmtAge(iso: string, now: Date = new Date()): string {
  const t = new Date(iso).getTime();
  if (Number.isNaN(t)) return iso;
  const diffMin = Math.max(0, Math.round((now.getTime() - t) / 60000));
  if (diffMin < 60) return `${diffMin} min ago`;
  const h = Math.round(diffMin / 60);
  if (h < 48) return `${h} h ago`;
  return `${Math.round(h / 24)} d ago`;
}

export function healthStrings(p: HealthParams): HealthStrings {
  switch (p.id) {
    case 'gamelog_missing':
      return {
        summary: 'No Game.log found — set a path in Settings to start the feed.',
      };
    case 'api_url_missing':
      return {
        summary: 'Remote sync is on but no API URL is set.',
      };
    case 'pair_missing':
      return {
        summary: 'This device isn’t paired with the StarStats server yet.',
      };
    case 'auth_lost':
      return {
        summary: 'This device is no longer paired — re-pair to resume syncing.',
      };
    case 'cookie_missing':
      return {
        summary: 'Hangar sync needs your RSI session cookie — paste it in Settings.',
      };
    case 'sync_failing':
      return {
        summary: 'Remote sync is failing.',
        // attempts_since_success is plumbed but not yet populated by
        // the snapshot path; render the bare error until that lands
        // so we don't display a misleading "(0 attempts since…)".
        detail:
          p.attempts_since_success > 0
            ? `${p.last_error} (${p.attempts_since_success} attempts since last success)`
            : p.last_error,
      };
    case 'hangar_skip':
      return {
        summary: 'Hangar sync skipped.',
        detail: `${p.reason} · ${fmtAge(p.since)}`,
      };
    case 'email_unverified':
      return {
        summary: 'Your email isn’t verified.',
      };
    case 'game_log_stale':
      return {
        summary: 'Game.log has been quiet while Star Citizen is running.',
        detail: `last event ${fmtAge(p.last_event_at)}`,
      };
    case 'update_available':
      return {
        summary: `Update available: v${p.version}.`,
      };
    case 'disk_free_low':
      return {
        summary: `Low disk space: ${fmtBytes(p.free_bytes)} free.`,
      };
  }
}
