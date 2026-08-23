import React from 'react';
import Link from 'next/link';
import type { Route } from 'next';
import { Plane, MeterRow, Flatline } from 'holo';
import { formatDwell, type DistinctStop } from '@/components/journey/trail-utils';
import { dwellAvailableAt, type DwellIndex } from './dwell';

/**
 * Where you have been, by taxonomy level.
 *
 * `Journey.jsx` browses locations through a category strip — Systems, Planets,
 * Cities, Sites — each carrying a count, with the selected level listing its
 * places ranked. The product had one fixed level (a system breakdown), so a
 * reader could see which systems they had visited and had no way to ask the
 * same question of planets or cities.
 *
 * NOTHING IS FETCHED FOR THIS. `DistinctStop` already carries `system`,
 * `planet` and `city`, and the stop itself is the site — the four levels are a
 * regrouping of data the page has, not a new query.
 *
 * RANKED BY DWELL, like the kit — where the endpoint has it.
 * `/v1/me/location/breakdown` aggregates real `dwell_seconds` per
 * system / planet / city, so those three levels rank by hours. A SITE is
 * deeper than the endpoint aggregates, so it falls back to visit count.
 *
 * The caption says which measure is on screen, because they are not
 * interchangeable: the place you visit most often and the place you spend most
 * of your time are routinely different, and a strip that silently swapped
 * between them per level would be its own kind of lie.
 *
 * (This file previously ranked everything by visits and said in a comment that
 * the product had no per-place dwell. It does, and always did.)
 *
 * URL-DRIVEN, like the rest of this page: the level is a query param, so a
 * chosen level is shareable and survives the back button. It is not client
 * state pretending to be navigation.
 */
export type TaxonomyLevel = 'system' | 'planet' | 'city' | 'site';

const LEVELS: readonly { id: TaxonomyLevel; label: string }[] = [
  { id: 'system', label: 'Systems' },
  { id: 'planet', label: 'Planets' },
  { id: 'city', label: 'Cities' },
  { id: 'site', label: 'Sites' },
];

/** The value a stop contributes at a level, or null when it is unplaced. */
function valueAt(stop: DistinctStop, level: TaxonomyLevel): string | null {
  if (level === 'system') return stop.system;
  if (level === 'planet') return stop.planet;
  if (level === 'city') return stop.city;
  // The site IS the stop. Prefer the classifier's friendly name over a raw
  // engine key, which is what `resolvedLabel` exists for.
  return stop.resolvedLabel ?? stop.label ?? null;
}

/** Visit counts per distinct value at one level, densest first. */
function tally(
  stops: DistinctStop[],
  level: TaxonomyLevel,
): { name: string; count: number }[] {
  const counts = new Map<string, number>();
  for (const s of stops) {
    const v = valueAt(s, level);
    if (!v) continue;
    counts.set(v, (counts.get(v) ?? 0) + 1);
  }
  return [...counts.entries()]
    .map(([name, count]) => ({ name, count }))
    .sort((a, b) => b.count - a.count || a.name.localeCompare(b.name));
}

export function TaxonomyStrip({
  stops,
  level,
  buildHref,
  buildPlaceHref,
  dwell,
}: {
  stops: DistinctStop[];
  level: TaxonomyLevel;
  /** Same-page link that swaps the level, preserving the range. */
  buildHref: (level: TaxonomyLevel) => string;
  /**
   * Link that opens one place's detail. `Journey.jsx` selects a row to
   * descend; the rows were inert here, so the screen could say you had been
   * somewhere 212 times and nothing about the place itself.
   */
  buildPlaceHref?: (place: string) => string;
  /** Real dwell per place, when the endpoint answered. */
  dwell?: DwellIndex;
}) {
  const byDwell = Boolean(dwell) && dwellAvailableAt(level);
  const seen = tally(stops, level);
  // Dwell ranking still only lists places THIS WINDOW'S TRACE contains, so the
  // strip never names somewhere the trail below does not show.
  const rows = byDwell
    ? seen
        .map((r) => ({
          ...r,
          dwellSeconds: dwell?.[level].get(r.name)?.dwellSeconds ?? 0,
        }))
        .sort(
          (a, b) =>
            b.dwellSeconds - a.dwellSeconds || a.name.localeCompare(b.name),
        )
    : seen.map((r) => ({ ...r, dwellSeconds: 0 }));
  const usableDwell = byDwell && rows.some((r) => r.dwellSeconds > 0);
  const peak = rows.length > 0 ? (usableDwell ? rows[0].dwellSeconds : rows[0].count) : 0;

  return (
    <>
      <nav className="hp-catstrip" aria-label="Location level">
        {LEVELS.map((l) => {
          const n = tally(stops, l.id).length;
          return (
            <Link
              key={l.id}
              href={buildHref(l.id) as Route}
              prefetch={false}
              className="hp-catchip"
              data-active={l.id === level ? 'true' : undefined}
              aria-current={l.id === level ? 'page' : undefined}
            >
              {l.label}
              {/* The count is part of the label here, as in the kit's strip:
                  knowing there are two systems and 179 sites is most of the
                  answer before you pick one. */}
              <b className="hp-catchip__n">{n}</b>
            </Link>
          );
        })}
      </nav>

      <Plane
        tilt="flat"
        cap={LEVELS.find((l) => l.id === level)?.label ?? 'Places'}
        hint={usableDwell ? 'by time spent' : 'by visits'}
        style={{ marginTop: 16 }}
      >
        {rows.length === 0 ? (
          <Flatline
            compact
            reason="no-data"
            title="Nothing placed at this level"
            hint="The catalogue could not resolve a name at this tier for any stop in this window."
          />
        ) : (
          rows
            .slice(0, 24)
            .map((r, i) => (
              <MeterRow
                key={r.name}
                rank={i + 1}
                name={
                  buildPlaceHref ? (
                    <Link href={buildPlaceHref(r.name) as Route}>{r.name}</Link>
                  ) : (
                    r.name
                  )
                }
                pct={
                  peak > 0
                    ? ((usableDwell ? r.dwellSeconds : r.count) / peak) * 100
                    : 0
                }
                value={
                  usableDwell ? formatDwell(r.dwellSeconds) : `${r.count}`
                }
              />
            ))
        )}
      </Plane>
      {rows.length > 24 ? (
        <p className="hp-prose">
          {/* The system's rule: a capped list announces its cap rather than
              trailing off as though it were complete. */}
          Showing the 24 most visited of {rows.length}.
        </p>
      ) : null}
    </>
  );
}
