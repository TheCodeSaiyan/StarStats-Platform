'use client';

import React from 'react';
import {
  CompareBar,
  CompareChart,
  type CompareStat,
} from 'holo';
import { fetchCompareVectors } from '@/lib/kb-compare';
import type { CompareEntry } from '@/lib/kb-compare-types';
import type { ReferenceCategory } from '@/lib/reference-types';

/**
 * Overlay comparison on the BROWSE screen — pick two or three entries from the
 * page you are already looking at and see the difference as a shape.
 *
 * THIS WAS MISSING ENTIRELY. `Catalogue.jsx` — one of the screens COVERAGE
 * marks as read from real source — puts this between the sort row and the card
 * grid, and the first pass at `/kb/[category]` shipped without it because the
 * spec'd screen was never opened. It is a feature of the catalogue, not
 * decoration on it: the reason to browse a list of ships is usually to weigh
 * two of them against each other, and the alternative is opening two tabs.
 *
 * WHERE THE DATA COMES FROM, AND WHY THE TIMING IS LOAD-BEARING.
 *
 * The browse page makes ZERO runtime API calls. Since the M10 cutover its
 * catalogue is built at BUILD time from the static `reference-data` snapshots
 * (see `lib/reference.ts`) — no listing fetch, no data cache, no rate limit.
 * This component is the only thing on the page that talks to the API at all.
 *
 * It resolves picks through `/kb/compare/{category}`, a same-origin proxy onto
 * the runtime reference API — which is **per-IP rate limited**, with the web
 * container fronting every request from a single IP. `lib/reference.ts` records
 * what that costs when it goes wrong: an escape hatch that disabled the cache
 * on per-slug reads turned a Next prefetch burst into a wave of uncached calls,
 * tripped the governor and 429'd.
 *
 * So: NOTHING IS FETCHED ON RENDER. The request fires only after a reader has
 * picked a second entry — a deliberate act, once, per reader who wants it. Do
 * not "optimise" this into a prefetch, an eager load of the visible page, or a
 * fetch on first pick; each of those turns one intentional request into one per
 * page view, on the busiest read surface in the product.
 *
 * AXES ARE PER CATEGORY and each carries its own scale, because a comparison
 * that normalises speed against mass is meaningless. `invert` marks a stat
 * where lower is better so a bigger shape always reads as stronger. The keys
 * are the metric paths the compare endpoint returns — the same ones the entity
 * sheet's radar uses, not a second vocabulary.
 */
const AXES: Partial<Record<ReferenceCategory, CompareStat[]>> = {
  vehicle: [
    { key: 'speed.scm', label: 'Speed', min: 0, max: 1400 },
    { key: 'agility.roll', label: 'Roll', min: 0, max: 200 },
    { key: 'agility.yaw', label: 'Yaw', min: 0, max: 60 },
    {
      key: 'weaponry.fixed_weapons.dps_total',
      label: 'Firepower',
      min: 0,
      max: 3000,
    },
    { key: 'health', label: 'Hull', min: 0, max: 60000 },
    { key: 'shield_hp', label: 'Shield', min: 0, max: 6000 },
  ],
  weapon: [
    {
      key: 'personal_weapon.damage.dps_total',
      label: 'DPS',
      min: 0,
      max: 260,
    },
    {
      key: 'personal_weapon.damage.alpha_total',
      label: 'Alpha',
      min: 0,
      max: 200,
    },
    { key: 'personal_weapon.rof', label: 'RoF', min: 0, max: 700 },
    {
      key: 'personal_weapon.effective_range',
      label: 'Range',
      min: 0,
      max: 1600,
    },
    {
      key: 'personal_weapon.ammunition.speed',
      label: 'Muzzle',
      min: 0,
      max: 1600,
    },
  ],
};

export function BrowseCompare({
  category,
  options,
}: {
  category: ReferenceCategory;
  /** The entries on this page, in render order. `{ slug, name }`. */
  options: { slug: string; name: string }[];
}) {
  const stats = AXES[category];
  const [picked, setPicked] = React.useState<string[]>([]);
  const [mode, setMode] = React.useState<'radar' | 'bars'>('radar');
  const [entries, setEntries] = React.useState<CompareEntry[]>([]);

  const bySlug = React.useMemo(() => {
    const m = new Map<string, string>();
    options.forEach((o) => m.set(o.name, o.slug));
    return m;
  }, [options]);

  React.useEffect(() => {
    if (picked.length < 2) {
      setEntries([]);
      return;
    }
    let cancelled = false;
    const slugs = picked.map((n) => bySlug.get(n)).filter(Boolean) as string[];
    fetchCompareVectors(category, slugs).then((r) => {
      if (!cancelled) setEntries(r.entries);
    });
    return () => {
      cancelled = true;
    };
  }, [category, picked, bySlug]);

  // A category with no axes has nothing to overlay — items and locations carry
  // no comparable numeric spine. Rendering an empty picker would advertise a
  // feature that cannot work here.
  if (!stats || options.length < 2) return null;

  const toggle = (name: string) =>
    setPicked((prev) =>
      prev.includes(name)
        ? prev.filter((n) => n !== name)
        : prev.length >= 3
          ? prev
          : [...prev, name],
    );

  const series = picked
    .map((name) => {
      const slug = bySlug.get(name);
      const e = entries.find((x) => x.slug === slug);
      return e ? { name, values: e.metrics } : null;
    })
    .filter(Boolean) as { name: string; values: Record<string, number> }[];

  const axisWords = stats.map((s) => s.label.toLowerCase());
  const axisSentence =
    axisWords.slice(0, -1).join(', ') + ' and ' + axisWords[axisWords.length - 1];

  return (
    <div className="hp-cmpwrap">
      <CompareBar
        options={options.slice(0, 8).map((o) => o.name)}
        selected={picked}
        onToggle={toggle}
        mode={mode}
        onMode={setMode}
      />
      {series.length >= 2 ? (
        <CompareChart mode={mode} stats={stats} series={series} />
      ) : (
        <p className="hp-prose">
          {picked.length >= 2
            ? 'Loading comparison…'
            : `Pick two or three to overlay them. Axes are ${axisSentence}.`}
        </p>
      )}
    </div>
  );
}
