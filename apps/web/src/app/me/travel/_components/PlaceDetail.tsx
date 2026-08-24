import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { Plane, MeterRow, SubStats, HoloKV, HoloTable, Flatline } from 'holo';
import {
  formatDwell,
  type DistinctStop,
} from '@/components/journey/trail-utils';
import type { TaxonomyLevel } from './TaxonomyStrip';
import { dwellAvailableAt, totalDwellAt, type DwellIndex } from './dwell';
import { RowLink } from '@/app/me/_projection/RowLink';

/**
 * One place, in depth — Journey's detail state.
 *
 * `Journey.jsx` runs `CatalogueLayout`'s three-part shell, and the port
 * shipped the first two: the level tabs and the ranked list. Selecting a row
 * did nothing, so the screen could tell you that you had been to Orison 212
 * times and nothing whatsoever about Orison. The spec's detail is a
 * `1.1fr / 1fr` grid — the left pane carries the figures and what is inside
 * this place, the right column stacks record panes.
 *
 * EVERYTHING HERE IS DERIVED FROM `DistinctStop[]`, WHICH THE PAGE ALREADY
 * HAS. No new endpoint, no new fetch: the level tabs were already a regrouping
 * of the same array, and so is this.
 *
 * DWELL IS REAL, and this file previously said it was not.
 * `/v1/me/location/breakdown` returns `dwell_seconds` per
 * system / planet / city, so `Dwell` and `Share` are the kit's own figures,
 * measured. A SITE is deeper than that endpoint aggregates, so at site level
 * the pane shows the sighting span instead and labels it as such — the span
 * between a stop's first and last sighting, which understates a visit and is
 * zero for a single-sighting stop, so it is never called dwell.
 *
 * WHAT THE KIT SHOWS THAT THIS CANNOT, and why the substitutes are named
 * differently rather than dressed up:
 *
 *   - `Deaths 9`. Not in the trace at all. Absent rather than zero: a zero
 *     would read as "you never died here", which is a claim this page cannot
 *     make.
 *   - `Arrived by: Constellation`. The trace carries no vehicle. The arrivals
 *     log keeps When and the sighting span and drops the column.
 *
 * `First seen` and `Last seen` ARE real, and are the two figures a reader most
 * often wants from a place they half-remember.
 */

/** The value a stop contributes at a level, or null when it is unplaced. */
function valueAt(stop: DistinctStop, level: TaxonomyLevel): string | null {
  if (level === 'system') return stop.system;
  if (level === 'planet') return stop.planet;
  if (level === 'city') return stop.city;
  return stop.resolvedLabel ?? stop.label ?? null;
}

const NEXT_LEVEL: Partial<Record<TaxonomyLevel, TaxonomyLevel>> = {
  system: 'planet',
  planet: 'city',
  city: 'site',
};

const LEVEL_NOUN: Record<TaxonomyLevel, string> = {
  system: 'system',
  planet: 'planet',
  city: 'city',
  site: 'site',
};

function shortDate(iso: string): string {
  // Fixed locale and UTC: this renders on the server and hydrates on the
  // client, and a locale-dependent date is a hydration mismatch.
  return new Date(iso).toLocaleDateString('en-GB', {
    day: '2-digit',
    month: 'short',
    year: 'numeric',
    timeZone: 'UTC',
  });
}

/** Whole-minute span between two ISO stamps, or null when it is zero. */
function spanMinutes(from: string, to: string): number | null {
  const ms = new Date(to).getTime() - new Date(from).getTime();
  if (!Number.isFinite(ms) || ms <= 0) return null;
  return Math.round(ms / 60000);
}

function humanSpan(minutes: number | null): string {
  if (minutes === null) return '—';
  if (minutes < 60) return `${minutes}m`;
  const h = Math.floor(minutes / 60);
  const m = minutes % 60;
  return m === 0 ? `${h}h` : `${h}h ${m}m`;
}

export function PlaceDetail({
  stops,
  level,
  place,
  buildChildHref,
  dwell,
}: {
  stops: DistinctStop[];
  level: TaxonomyLevel;
  place: string;
  /** Link that descends to a child place at the next level. */
  buildChildHref: (level: TaxonomyLevel, place: string) => string;
  /** Real dwell per place, when the endpoint answered. */
  dwell?: DwellIndex;
}) {
  const here = stops.filter((s) => valueAt(s, level) === place);

  if (here.length === 0) {
    return (
      <Flatline
        reason="no-data"
        title={`No visits to ${place} in this window`}
        hint="Widen the range, or pick another place from the list."
      />
    );
  }

  const totalAtLevel = stops.filter((s) => valueAt(s, level) !== null).length;
  const visits = here.length;
  const events = here.reduce((n, s) => n + s.eventCount, 0);
  const sharePct = totalAtLevel > 0 ? (visits / totalAtLevel) * 100 : 0;

  const sorted = [...here].sort((a, b) => a.enteredAt.localeCompare(b.enteredAt));
  const firstSeen = sorted[0].enteredAt;
  const lastSeen = sorted[sorted.length - 1].lastSeenAt;

  // Total sighting span across every visit. Named for what it measures — see
  // the note above on why this is not dwell.
  const spanTotal = here.reduce(
    (n, s) => n + (spanMinutes(s.enteredAt, s.lastSeenAt) ?? 0),
    0,
  );

  const next = NEXT_LEVEL[level];
  const children: { name: string; count: number }[] = [];
  if (next) {
    const counts = new Map<string, number>();
    for (const s of here) {
      const v = valueAt(s, next);
      if (!v) continue;
      counts.set(v, (counts.get(v) ?? 0) + 1);
    }
    children.push(
      ...[...counts.entries()]
        .map(([name, count]) => ({ name, count }))
        .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name)),
    );
  }
  const peak = children.length > 0 ? children[0].count : 0;

  const parent =
    level === 'system'
      ? '—'
      : level === 'planet'
        ? (here[0].system ?? '—')
        : level === 'city'
          ? (here[0].planet ?? here[0].system ?? '—')
          : (here[0].city ?? here[0].planet ?? '—');

  // Dwell, where the endpoint aggregates it. `Share` follows whichever measure
  // is on screen so the percentage and the figure beside it agree — a share of
  // dwell next to a visit count would be two different denominators presented
  // as one row.
  const totals = dwellAvailableAt(level) ? dwell?.[level].get(place) : undefined;
  const dwellSeconds = totals?.dwellSeconds ?? 0;
  const dwellTotal = dwellAvailableAt(level)
    ? (dwell ? totalDwellAt(dwell, level) : 0)
    : 0;
  const hasDwell = dwellSeconds > 0 && dwellTotal > 0;

  const figures = [
    { k: 'Visits', v: visits.toLocaleString('en-GB') },
    hasDwell
      ? { k: 'Dwell', v: formatDwell(dwellSeconds) }
      : { k: 'Sighting span', v: humanSpan(spanTotal || null) },
    {
      k: 'Share',
      v: Math.round(
        hasDwell ? (dwellSeconds / dwellTotal) * 100 : sharePct,
      ).toString(),
      u: '%',
    },
    { k: 'Events', v: events.toLocaleString('en-GB') },
  ];

  const arrivals = [...here]
    .sort((a, b) => b.enteredAt.localeCompare(a.enteredAt))
    .slice(0, 8);

  return (
    <div className="hp-journeydetail">
      <div>
        <SubStats items={figures} />

        <Plane
          cap={<h3>{next ? `Within this ${LEVEL_NOUN[level]}` : 'Inside'}</h3>}
          hint={next ? `descend to ${LEVEL_NOUN[next]}` : 'deepest level'}
          style={{ marginTop: 22 }}
        >
          {children.length > 0 ? (
            children.slice(0, 12).map((c, i) => (
              <MeterRow
                key={c.name}
                rank={i + 1}
                // The ROW carries the href, so the whole row is the target.
                // Wrapping the label instead left a 33-58px anchor in a
                // full-width row — the fault reported on /me's planes.
                name={c.name}
                href={next ? buildChildHref(next, c.name) : undefined}
                linkAs={RowLink}
                pct={peak > 0 ? (c.count / peak) * 100 : 0}
                value={`${c.count}`}
              />
            ))
          ) : (
            <Flatline
              compact
              reason="no-data"
              title={
                next
                  ? `Nothing charted inside ${place}`
                  : 'This is the deepest level the taxonomy has'
              }
              hint={
                next
                  ? 'Stops here carried no place name at the next level down.'
                  : 'A site has nothing beneath it.'
              }
            />
          )}
        </Plane>
      </div>

      <div className="hp-journeyrecords">
        <Plane tilt="flat" cap={<h3>Scope record</h3>} hint="from the taxonomy">
          <HoloKV
            items={[
              {
                k: 'Level',
                // Capitalised: it is a record VALUE beside "Parent" and
                // "First seen", not the mid-sentence noun the plane cap uses.
                v: LEVEL_NOUN[level][0].toUpperCase() + LEVEL_NOUN[level].slice(1),
              },
              { k: 'Parent', v: parent },
              {
                k: 'Charted inside',
                v: next ? String(children.length) : '—',
              },
              { k: 'First seen', v: shortDate(firstSeen) },
              { k: 'Last seen', v: shortDate(lastSeen) },
            ]}
          />
        </Plane>

        <Plane tilt="flat" cap={<h3>Arrivals</h3>} hint="most recent">
          <HoloTable
            columns={[
              { key: 'when', label: 'When' },
              { key: 'at', label: 'Seen at' },
              { key: 'span', label: 'Span', numeric: true },
            ]}
            rows={arrivals.map((s, i) => ({
              key: `${s.key}:${i}`,
              when: shortDate(s.enteredAt),
              at: s.resolvedLabel ?? s.label,
              span: humanSpan(spanMinutes(s.enteredAt, s.lastSeenAt)),
            }))}
          />
        </Plane>
      </div>
    </div>
  );
}
