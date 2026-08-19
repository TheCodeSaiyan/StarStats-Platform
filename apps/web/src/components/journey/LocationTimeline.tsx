/**
 * Vertical timeline of distinct stops. Replaces the inline ordered-
 * list rendering on /journey Location, AND absorbs the role the
 * now-removed `LocationConstellation` was meant to serve — system
 * grouping is now an inline marker between rows when the stream
 * crosses systems.
 *
 * Server component. Newest-at-top. Long tails collapse behind a
 * `<details>` disclosure so the card stays scannable.
 */

import React from 'react';
import type { TraceEntry } from '@/lib/api';
import type { ReferenceCatalog } from '@/lib/reference-types';
import { EntityLink } from '@/components/kb/EntityLink';
import {
  type DistinctStop,
  toDistinctStops,
  glyphFor,
  formatDwell,
  relativeAge,
} from './trail-utils';

interface Props {
  entries: TraceEntry[];
  /** Hard cap on rendered rows even when the disclosure is open. */
  maxStops?: number;
  /** Rows visible without expanding the disclosure. Default 8. */
  initialStops?: number;
  /** Optional locations catalog. When supplied, each stop's label
   *  links to `/kb/location/{slug}` with the EntityHoverCard popover
   *  on hover. Pass `catalogs.locations` from the page's
   *  `loadAllReferenceBundles()` call. */
  catalog?: ReferenceCatalog;
}

export function LocationTimeline({
  entries,
  maxStops = 40,
  initialStops = 8,
  catalog,
}: Props) {
  const stops = toDistinctStops(entries).slice(-maxStops).reverse();

  if (stops.length === 0) {
    return (
      <p style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13 }}>
        No location-bearing events in the window.
      </p>
    );
  }

  const visibleCount = Math.min(stops.length, initialStops);
  const head = stops.slice(0, visibleCount);
  const tail = stops.slice(visibleCount);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 0 }}>
      <Timeline rows={head} indexOffset={0} catalog={catalog} />
      {tail.length > 0 && (
        <details
          style={{
            marginTop: 4,
            borderTop: '1px dashed var(--border)',
            paddingTop: 8,
          }}
        >
          <summary
            style={{
              cursor: 'pointer',
              listStyle: 'none',
              fontSize: 12,
              color: 'var(--accent)',
              padding: '4px 0 8px',
            }}
          >
            Show {tail.length} earlier stop{tail.length === 1 ? '' : 's'}
          </summary>
          <Timeline rows={tail} indexOffset={head.length} catalog={catalog} />
        </details>
      )}
    </div>
  );
}

function Timeline({
  rows,
  indexOffset,
  catalog,
}: {
  rows: DistinctStop[];
  indexOffset: number;
  catalog?: ReferenceCatalog;
}) {
  return (
    <ol
      style={{
        listStyle: 'none',
        margin: 0,
        padding: 0,
        position: 'relative',
        display: 'flex',
        flexDirection: 'column',
        gap: 0,
      }}
    >
      <span
        aria-hidden
        style={{
          position: 'absolute',
          left: 9,
          top: 14,
          bottom: 14,
          width: 1,
          background: 'var(--border)',
        }}
      />
      {rows.map((stop, i) => {
        const globalIdx = indexOffset + i;
        // Boundary marker: the NEXT row (older in chronological time,
        // since we render newest-first) sits in a different system.
        // Render a thin "from → to" marker BELOW this row when so.
        const nextRow = rows[i + 1];
        const systemChanged =
          nextRow != null &&
          (stop.system ?? null) !== (nextRow.system ?? null);
        return (
          <li key={stop.key + stop.enteredAt}>
            <TimelineRow
              stop={stop}
              isLatest={globalIdx === 0}
              catalog={catalog}
            />
            {systemChanged && (
              <SystemChangeMarker
                from={nextRow.system ?? '—'}
                to={stop.system ?? '—'}
                catalog={catalog}
              />
            )}
          </li>
        );
      })}
    </ol>
  );
}

function TimelineRow({
  stop,
  isLatest,
  catalog,
}: {
  stop: DistinctStop;
  isLatest: boolean;
  catalog?: ReferenceCatalog;
}) {
  // `stop.label` is computed by `primaryLabel()` as city ?? planet
  // ?? system ?? 'In transit' — when it's not the literal "In
  // transit" fallback, it IS one of the raw entity fields, so it's
  // the right classKey for the displayed text. Same shape for
  // `stop.sublabel` via `secondaryLabel()` (raw planet or system,
  // or null). Reading from the canonical label fields keeps the
  // classKey aligned with what the user sees — using a separate
  // city/planet/system precedence drifts apart in edge cases (e.g.
  // sublabel = system while planet is also present).
  const classKey = stop.label === 'In transit' ? null : stop.label;
  const sublabelKey = stop.sublabel;
  const dwellSec = dwellSeconds(stop);
  return (
    <div
      style={{
        position: 'relative',
        display: 'grid',
        gridTemplateColumns: '20px 1fr auto',
        gap: 12,
        alignItems: 'baseline',
        padding: '10px 0',
      }}
    >
      <span
        aria-hidden
        style={{
          display: 'inline-block',
          width: 10,
          height: 10,
          marginTop: 6,
          borderRadius: '50%',
          background: isLatest ? 'var(--accent)' : 'var(--fg-dim)',
          boxShadow: '0 0 0 3px var(--bg)',
          marginLeft: 4,
        }}
      />
      <div style={{ minWidth: 0 }}>
        <div
          style={{
            display: 'flex',
            alignItems: 'baseline',
            gap: 8,
            fontSize: 14,
            color: 'var(--fg)',
          }}
        >
          <span aria-hidden style={{ fontSize: 13, opacity: 0.85 }}>
            {glyphFor(stop)}
          </span>
          <strong style={{ fontWeight: isLatest ? 600 : 500 }}>
            <EntityLink
              category="location"
              classKey={classKey}
              catalog={catalog}
              label={stop.label}
              resolvedLabel={stop.resolvedLabel}
              resolvedSlug={stop.resolvedSlug}
            />
          </strong>
          {stop.sublabel && (
            <span style={{ color: 'var(--fg-muted)', fontSize: 12 }}>
              {' · '}
              <EntityLink
                category="location"
                classKey={sublabelKey}
                catalog={catalog}
                label={stop.sublabel}
              />
            </span>
          )}
        </div>
        <div
          style={{
            marginTop: 2,
            fontSize: 11,
            color: 'var(--fg-dim)',
            display: 'flex',
            gap: 10,
            flexWrap: 'wrap',
          }}
        >
          <span className="mono" title={stop.enteredAt}>
            entered {relativeAge(stop.enteredAt)} ago
          </span>
          {dwellSec > 0 && (
            <span
              className="mono"
              title="time between first and last event at this stop"
            >
              · dwell {formatDwell(dwellSec)}
            </span>
          )}
          <span className="mono">
            · {stop.eventCount} event{stop.eventCount === 1 ? '' : 's'}
          </span>
        </div>
      </div>
    </div>
  );
}

function SystemChangeMarker({
  from,
  to,
  catalog,
}: {
  from: string;
  to: string;
  catalog?: ReferenceCatalog;
}) {
  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        marginLeft: 18,
        padding: '4px 0',
        fontSize: 10,
        color: 'var(--fg-dim)',
        textTransform: 'uppercase',
        letterSpacing: '0.08em',
        fontFamily: 'var(--font-mono, ui-monospace, monospace)',
      }}
    >
      <span
        aria-hidden
        style={{ flex: '0 0 24px', borderTop: '1px dashed var(--border)' }}
      />
      <span>
        <EntityLink
          category="location"
          classKey={from}
          catalog={catalog}
          label={from}
        />{' '}
        <span aria-hidden>→</span>{' '}
        <EntityLink
          category="location"
          classKey={to}
          catalog={catalog}
          label={to}
        />
      </span>
      <span
        aria-hidden
        style={{ flex: 1, borderTop: '1px dashed var(--border)' }}
      />
    </div>
  );
}

function dwellSeconds(stop: DistinctStop): number {
  const a = new Date(stop.enteredAt).getTime();
  const b = new Date(stop.lastSeenAt).getTime();
  if (Number.isNaN(a) || Number.isNaN(b) || b <= a) return 0;
  return Math.round((b - a) / 1000);
}
