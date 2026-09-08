import type { RecentActivityEvent } from '@/app/_components/widgets/recent_activity';

/**
 * The fields a log row opens into: where the event came from, then every
 * field the payload carries, verbatim. The sentence above the row already
 * interprets the event; this is the record behind it, so values are not
 * prettified — a reader opening a row wants what the log said.
 */
export interface DetailItem {
  k: string;
  v: string;
}

const MISSING = '—';

function label(key: string): string {
  const words = key.replace(/_/g, ' ').trim();
  return words.charAt(0).toUpperCase() + words.slice(1);
}

function value(v: unknown): string {
  if (v === null || v === undefined || v === '') return MISSING;
  if (typeof v === 'boolean') return v ? 'yes' : 'no';
  if (typeof v === 'string' || typeof v === 'number') return String(v);
  return JSON.stringify(v);
}

export function eventDetailItems(e: RecentActivityEvent): DetailItem[] {
  const items: DetailItem[] = [
    { k: 'Type', v: e.event_type },
    { k: 'Logged', v: e.event_timestamp ?? MISSING },
    { k: 'Source', v: e.log_source ?? MISSING },
    { k: 'Sequence', v: e.seq === undefined ? MISSING : String(e.seq) },
  ];
  if (e.resolved_location) {
    const loc = e.resolved_location;
    items.push({
      k: 'Location',
      v: loc.system ? `${loc.display_name} · ${loc.system}` : loc.display_name,
    });
  }
  if (typeof e.payload === 'object' && e.payload !== null) {
    for (const [key, v] of Object.entries(e.payload as Record<string, unknown>)) {
      if (key === 'type' || key === 'timestamp') continue;
      items.push({ k: label(key), v: value(v) });
    }
  }
  return items;
}
