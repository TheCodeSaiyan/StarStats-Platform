import Link from 'next/link';
import type { HangarSnapshot } from '@/lib/api';

/**
 * Hangar snapshot card. Server-component-only — purely presentational.
 *
 * The tray client scrapes the user's RSI website pledges page and POSTs
 * a snapshot to `/v1/me/hangar`; this card renders the most recent one
 * without launching the tray. The empty state nudges users who haven't
 * paired the tray yet, since that's the only writer.
 *
 * No inline "Refresh now" button (unlike ProfileCard / OrgsCard): the
 * server holds zero RSI credentials by design — only the tray, running
 * on the user's machine and reading the RSI cookie out of the OS
 * keychain, can pull a fresh pledges page without breaking that trust
 * boundary. So the affordance is a tray-launch hint ("Updated via tray
 * · open Devices") rather than a Server Action — the truthful framing
 * of where the refresh actually happens.
 */

const SHIP_PREVIEW_LIMIT = 6;

function formatRelative(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return iso;
  const deltaSec = Math.round((then - Date.now()) / 1000);
  const rtf = new Intl.RelativeTimeFormat('en', { numeric: 'auto' });
  const abs = Math.abs(deltaSec);
  if (abs < 60) return rtf.format(deltaSec, 'second');
  if (abs < 3600) return rtf.format(Math.round(deltaSec / 60), 'minute');
  if (abs < 86_400) return rtf.format(Math.round(deltaSec / 3600), 'hour');
  if (abs < 30 * 86_400) {
    return rtf.format(Math.round(deltaSec / 86_400), 'day');
  }
  if (abs < 365 * 86_400) {
    return rtf.format(Math.round(deltaSec / (30 * 86_400)), 'month');
  }
  return rtf.format(Math.round(deltaSec / (365 * 86_400)), 'year');
}

/**
 * Group ships by their free-form `kind` field. RSI's classification
 * drifts (ship / ground vehicle / skin / upgrade / paint), so we treat
 * unknowns as a separate bucket rather than collapsing into "other"
 * — the user gets to see whatever shape RSI decided to ship today.
 */
function summariseByKind(ships: HangarSnapshot['ships']): Array<[string, number]> {
  const tally = new Map<string, number>();
  for (const ship of ships) {
    const key = ship.kind?.trim() || 'unspecified';
    tally.set(key, (tally.get(key) ?? 0) + 1);
  }
  return [...tally.entries()].sort((a, b) => b[1] - a[1]);
}

export function HangarCard({
  snapshot,
}: {
  snapshot: HangarSnapshot | null;
}) {
  if (!snapshot) {
    return (
      <section className="ss-card ss-card-pad">
        <div className="ss-eyebrow" style={{ marginBottom: 6 }}>
          Hangar
        </div>
        <h2
          style={{
            margin: 0,
            fontSize: 17,
            fontWeight: 600,
            letterSpacing: '-0.01em',
          }}
        >
          No hangar synced yet
        </h2>
        <p
          style={{
            margin: '8px 0 0',
            color: 'var(--fg-muted)',
            fontSize: 13,
            lineHeight: 1.55,
          }}
        >
          The StarStats tray reads your fleet from your RSI account when
          you pair it.{' '}
          <Link href="/devices" style={{ color: 'var(--accent)' }}>
            Pair a device
          </Link>{' '}
          to start the sync.
        </p>
      </section>
    );
  }

  const breakdown = summariseByKind(snapshot.ships);
  const preview = snapshot.ships.slice(0, SHIP_PREVIEW_LIMIT);
  const remaining = snapshot.ships.length - preview.length;

  return (
    <section className="ss-card ss-card-pad">
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'baseline',
          gap: 12,
          marginBottom: 6,
        }}
      >
        <div className="ss-eyebrow">Hangar</div>
        {/* Truthful refresh affordance — the tray is the only writer
            (RSI cookie sits in the OS keychain), so the link points at
            the device-pairing surface rather than promising a
            web-side refresh that can't exist. */}
        <Link
          href="/devices"
          style={{
            fontSize: 11,
            color: 'var(--fg-muted)',
            textDecoration: 'none',
          }}
        >
          Updated via tray · open Devices →
        </Link>
      </div>
      <h2
        style={{
          margin: 0,
          fontSize: 17,
          fontWeight: 600,
          letterSpacing: '-0.01em',
          marginBottom: 16,
        }}
      >
        Fleet snapshot
      </h2>

      <dl
        className="ss-kv"
        style={{ gridTemplateColumns: '160px 1fr', marginBottom: 16 }}
      >
        <dt>Last fetched</dt>
        <dd className="mono">{formatRelative(snapshot.captured_at)}</dd>
        <dt>Total items</dt>
        <dd className="mono">{snapshot.ships.length}</dd>
        {breakdown.length > 0 && (
          <>
            <dt>Breakdown</dt>
            <dd className="mono" style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
              {breakdown.map(([kind, count]) => (
                <span
                  key={kind}
                  className="ss-badge"
                  style={{ fontVariant: 'tabular-nums' }}
                >
                  {kind}: {count}
                </span>
              ))}
            </dd>
          </>
        )}
      </dl>

      {preview.length > 0 && (
        <ul
          style={{
            listStyle: 'none',
            margin: 0,
            padding: 0,
            display: 'flex',
            flexDirection: 'column',
            gap: 4,
            fontSize: 13,
            color: 'var(--fg-muted)',
          }}
        >
          {preview.map((ship, idx) => (
            <li
              key={(ship.pledge_id ?? ship.name) + '-' + idx}
              style={{
                display: 'flex',
                justifyContent: 'space-between',
                gap: 12,
              }}
            >
              <span style={{ color: 'var(--fg)' }}>{ship.name}</span>
              {ship.manufacturer && (
                <span className="mono">{ship.manufacturer}</span>
              )}
            </li>
          ))}
          {remaining > 0 && (
            <li
              style={{
                color: 'var(--fg-dim)',
                fontStyle: 'italic',
                fontSize: 12,
              }}
            >
              +{remaining} more
            </li>
          )}
        </ul>
      )}
    </section>
  );
}
