import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { HoloKV, BeamChip, Flatline } from 'holo';
import type { HangarSnapshot } from '@/lib/api';

/**
 * Fleet snapshot, in the projection.
 *
 * ARCHITECTURE INVARIANT, unchanged by the port: the server holds ZERO RSI
 * credentials. Only the tray scrapes the pledges page, using the user's own
 * cookie out of the OS keychain. So there is no "Refresh" here and there never
 * can be — the affordance points at `/downloads`, where pairing lives. Do not
 * "improve" this into a web-side refresh button; it would be a promise the
 * architecture cannot keep.
 *
 * Logic (relative time, per-kind tally, the six-item preview) is lifted from
 * `HangarCard` unchanged; only the drawing is new.
 */
const SHIP_PREVIEW_LIMIT = 6;

function formatRelative(iso: string): string {
  const then = new Date(iso).getTime();
  if (Number.isNaN(then)) return 'unknown';
  const deltaSec = Math.round((then - Date.now()) / 1000);
  const rtf = new Intl.RelativeTimeFormat('en', { numeric: 'auto' });
  const abs = Math.abs(deltaSec);
  if (abs < 60) return rtf.format(deltaSec, 'second');
  if (abs < 3600) return rtf.format(Math.round(deltaSec / 60), 'minute');
  if (abs < 86400) return rtf.format(Math.round(deltaSec / 3600), 'hour');
  return rtf.format(Math.round(deltaSec / 86400), 'day');
}

function summariseByKind(
  ships: HangarSnapshot['ships'],
): Array<[string, number]> {
  const tally = new Map<string, number>();
  for (const ship of ships) {
    const key = ship.kind?.trim() || 'unspecified';
    tally.set(key, (tally.get(key) ?? 0) + 1);
  }
  return [...tally.entries()].sort((a, b) => b[1] - a[1]);
}

export function HangarPane({ snapshot }: { snapshot: HangarSnapshot | null }) {
  if (!snapshot) {
    return (
      <>
        <Flatline
          title="No hangar synced yet"
          reason="no-signal"
          hint="The StarStats tray reads your fleet from your RSI account when you pair it."
          action={
            <Link href={'/downloads' as Route} className="hp-btn hp-btn--ghost">
              Pair a device →
            </Link>
          }
        />
      </>
    );
  }

  const breakdown = summariseByKind(snapshot.ships);
  const preview = snapshot.ships.slice(0, SHIP_PREVIEW_LIMIT);
  const remaining = snapshot.ships.length - preview.length;

  return (
    <>
      <div style={{ marginTop: 16 }}>
        <HoloKV
          items={[
            { k: 'Last fetched', v: formatRelative(snapshot.captured_at) },
            { k: 'Total items', v: String(snapshot.ships.length) },
            ...(breakdown.length > 0
              ? [
                  {
                    k: 'Breakdown',
                    v: (
                      <span
                        style={{
                          display: 'flex',
                          flexWrap: 'wrap',
                          gap: 6,
                        }}
                      >
                        {breakdown.map(([kind, count]) => (
                          <BeamChip key={kind}>
                            {kind}: {count}
                          </BeamChip>
                        ))}
                      </span>
                    ),
                  },
                ]
              : []),
            ...(preview.length > 0
              ? [
                  {
                    k: 'Fleet',
                    v: (
                      <span>
                        {preview.map((s) => s.name).join(' · ')}
                        {remaining > 0 ? ` · +${remaining} more` : ''}
                      </span>
                    ),
                  },
                ]
              : []),
          ]}
        />
      </div>
      <p className="hp-prose">
        Updated via the tray —{' '}
        {/* "open Hangar" pointed at `/devices`, which was the PAIRED-DEVICE
            page wearing the hangar's name. This pane IS the hangar; the link
            goes where the tray is managed. */}
        <Link href={'/downloads' as Route}>manage the emitter →</Link>
      </p>
    </>
  );
}
