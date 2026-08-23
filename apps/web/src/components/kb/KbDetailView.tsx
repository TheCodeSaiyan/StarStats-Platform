'use client';

import { Plane, BeamChip, seriesColor, SERIES_SLOTS } from 'holo';
import React, { useEffect, useMemo, useState } from 'react';
import type { ReferenceCategory, CohortRef } from '@/lib/reference-types';
import { type StatsGroups } from '@/lib/kb-stats-types';
import { useKbPrefs, type KbPrefs } from '@/lib/kb-prefs';
import { buildVisualModel } from '@/lib/kb-view-model';
import { buildDetailGroups } from '@/lib/kb-detail';
import { StatBar } from './StatBar';
import { HandlingRadar } from './HandlingRadar';
import { HeadlineCallouts } from './HeadlineCallouts';
import { DetailGroups } from './DetailGroups';
import { ComparisonTray, type SelectedShip } from './ComparisonTray';
import { ComparisonRadar } from './ComparisonRadar';
import { ComparisonMatrix } from './ComparisonMatrix';
import { ComparisonLeaderboard } from './ComparisonLeaderboard';
import { fetchCompareVectors } from '@/lib/kb-compare';
import { fetchCohortMembers } from '@/lib/kb-cohort';
import {
  buildComparisonMatrix, buildComparisonRadar, buildLeaderboard,
  type CompareEntry, type SortSpec,
} from '@/lib/kb-compare-types';

// The sanctioned series palette, not a hardcoded ramp. The nine hex values
// this replaces ignored the calibration entirely: a comparison chart stayed
// teal-and-violet on Pyro, which is the one place in the product where colour
// is load-bearing and it was the one place that did not follow the beam.
const SERIES_COLORS = Array.from({ length: SERIES_SLOTS }, (_, i) =>
  seriesColor(i),
);

const RADAR_KEYS: Record<string, string[]> = {
  vehicle: ['speed.scm', 'agility.roll', 'agility.yaw', 'weaponry.fixed_weapons.dps_total', 'health', 'shield_hp'],
  weapon: [
    'personal_weapon.damage.dps_total',
    'personal_weapon.damage.alpha_total',
    'personal_weapon.rof',
    'personal_weapon.effective_range',
    'personal_weapon.ammunition.speed',
  ],
};
const RADAR_LABELS: Record<string, string> = {
  'speed.scm': 'Speed', 'agility.roll': 'Roll', 'agility.yaw': 'Yaw',
  'weaponry.fixed_weapons.dps_total': 'Firepower', health: 'Hull', shield_hp: 'Shield',
  'personal_weapon.damage.dps_total': 'DPS', 'personal_weapon.damage.alpha_total': 'Alpha',
  'personal_weapon.rof': 'RoF', 'personal_weapon.effective_range': 'Range',
  'personal_weapon.ammunition.speed': 'Speed',
};

// Per-category default sort for the comparison matrix. Vehicles sort by
// SCM speed; the other categories sort by their headline metric (so the
// matrix opens on a meaningful column instead of a vehicle-only path that
// doesn't exist for them).
const DEFAULT_SORT: Record<string, SortSpec> = {
  vehicle: { key: 'speed.scm', dir: 'desc' },
  weapon: { key: 'personal_weapon.damage.dps_total', dir: 'desc' },
  item: { key: 'mass', dir: 'desc' },
  location: { key: 'mission_count', dir: 'desc' },
};

export interface KbDetailViewProps {
  category: ReferenceCategory;
  displayName: string;
  metadata: Record<string, unknown>;
  groups: StatsGroups;
  cohorts: CohortRef[];
  description?: string;
  roleTags?: string[];
  serverPrefs: Partial<KbPrefs> | null;
  signedIn: boolean;
  anchorSlug: string;
  catalog: import('./ComparisonTray').CatalogItem[];
}

export function KbDetailView(props: KbDetailViewProps) {
  const { prefs, update } = useKbPrefs({ serverPrefs: props.serverPrefs, signedIn: props.signedIn });
  const [compareKey, setCompareKey] = useState(props.cohorts[0]?.key ?? '__all__');

  const visual = useMemo(() => {
    const bucket = props.groups[compareKey] ?? props.groups['__all__'] ?? {};
    return buildVisualModel(props.category, props.metadata, bucket, prefs.units);
  }, [props.category, props.metadata, props.groups, compareKey, prefs.units]);
  const compact = useMemo(
    () => buildDetailGroups(props.category, props.metadata),
    [props.category, props.metadata],
  );

  const [selectedSlugs, setSelectedSlugs] = useState<string[]>([]);
  const [onRadar, setOnRadar] = useState<Record<string, boolean>>({});
  const [vectors, setVectors] = useState<CompareEntry[]>([]);
  const [showComparison, setShowComparison] = useState(true);
  const [sort, setSort] = useState<SortSpec>(DEFAULT_SORT[props.category] ?? { key: 'speed.scm', dir: 'desc' });
  const [cohortNotice, setCohortNotice] = useState<string | null>(null);

  const comparing = selectedSlugs.length > 0;

  useEffect(() => {
    if (selectedSlugs.length === 0) { setVectors([]); return; }
    let cancelled = false;
    const slugs = [props.anchorSlug, ...selectedSlugs];
    fetchCompareVectors(props.category, slugs).then((r) => {
      if (!cancelled) setVectors(r.entries);
    });
    return () => { cancelled = true; };
  }, [props.category, props.anchorSlug, selectedSlugs]);

  const addShip = (slug: string) => {
    setSelectedSlugs((prev) => (prev.includes(slug) || prev.length >= 9 ? prev : [...prev, slug]));
    setOnRadar((prev) => ({ ...prev, [slug]: Object.keys(prev).filter((k) => prev[k]).length < 5 }));
  };
  const removeShip = (slug: string) => {
    setSelectedSlugs((prev) => prev.filter((s) => s !== slug));
    setOnRadar((prev) => {
      const next = { ...prev };
      delete next[slug];
      return next;
    });
  };
  const toggleRadar = (slug: string) => setOnRadar((prev) => ({ ...prev, [slug]: !prev[slug] }));

  const handleAddCohort = async (key: string) => {
    const res = await fetchCohortMembers(props.category, key);
    const room = 9 - selectedSlugs.length; // anchor + up to 9 others = 10
    const candidates = res.entries
      .map((e) => e.slug)
      .filter((s) => s !== props.anchorSlug && !selectedSlugs.includes(s));
    const toAdd = candidates.slice(0, Math.max(0, room));
    // Surface the cap to the user instead of silently dropping excess members;
    // addShip also enforces the 9-others hard cap as a secondary guard.
    setCohortNotice(
      toAdd.length < candidates.length
        ? `Added ${toAdd.length} of ${candidates.length} — comparison holds 10 ships.`
        : null,
    );
    toAdd.forEach(addShip);
  };

  const selectedChips: SelectedShip[] = selectedSlugs.map((slug, i) => ({
    slug,
    name: props.catalog.find((c) => c.slug === slug)?.display_name ?? slug,
    color: SERIES_COLORS[i % SERIES_COLORS.length],
    onRadar: onRadar[slug] ?? true,
  }));

  return (
    <>
      <ViewToggle view={prefs.view} units={prefs.units} onChange={update} />
      {prefs.view === 'visual' ? (
        <>
          <ComparisonTray
            anchorSlug={props.anchorSlug}
            anchorName={props.displayName}
            selected={selectedChips}
            catalog={props.catalog}
            max={10}
            onAdd={addShip}
            onRemove={removeShip}
            onToggleRadar={toggleRadar}
            cohorts={props.cohorts}
            onAddCohort={handleAddCohort}
          />

          {cohortNotice && (
            <p style={{ fontSize: 12, color: 'var(--fg-muted)', margin: 0 }} role="status">{cohortNotice}</p>
          )}

          {comparing && (
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button
                type="button"
                aria-pressed={showComparison}
                className="hp-toggle"
                data-active={showComparison ? 'true' : undefined}
                onClick={() => setShowComparison(true)}
              >
                Comparison
              </button>
              <button
                type="button"
                aria-pressed={!showComparison}
                className="hp-toggle"
                data-active={!showComparison ? 'true' : undefined}
                onClick={() => setShowComparison(false)}
              >
                Single
              </button>
            </div>
          )}

          {comparing && showComparison ? (
            vectors.length === 0 ? (
              <Plane tilt="flat" cap="Comparison" style={{ marginTop: 16 }}>
                <p className="hp-prose">Loading comparison…</p>
              </Plane>
            ) : (
              (() => {
                const anchor = vectors.find((v) => v.slug === props.anchorSlug);
                const others = vectors.filter((v) => v.slug !== props.anchorSlug);
                if (!anchor) return null;
                // The anchor is `--hot`, deliberately outside the palette: the entity whose
                // page this is should outshine everything it is measured against.
                const colorBySlug = new Map<string, string>([
                  [anchor.slug, seriesColor(0, true)],
                ]);
                selectedChips.forEach((c) => colorBySlug.set(c.slug, c.color));
                const matrix = buildComparisonMatrix(props.category, anchor, others, prefs.units, sort);
                const leaderboard = buildLeaderboard(props.category, vectors, prefs.units);
                const radarShips = [anchor, ...others.filter((o) => onRadar[o.slug] ?? true)].slice(0, 6);
                const axisKeys = (RADAR_KEYS[props.category] ?? []).filter((k) => radarShips.some((s) => typeof s.metrics[k] === 'number'));
                const radarModel = buildComparisonRadar(radarShips, axisKeys);
                return (
                  <>
                    <ComparisonLeaderboard cards={leaderboard} />
                    {radarModel.axes.length >= 3 && (
                      <Plane tilt="flat" cap="Handling" style={{ marginTop: 16 }}>
                        <ComparisonRadar
                          axisLabels={radarModel.axes.map((k) => RADAR_LABELS[k] ?? k)}
                          series={radarModel.series.map((s, i) => ({
                            ...s,
                            color: colorBySlug.get(s.slug) ?? seriesColor(i),
                          }))}
                        />
                      </Plane>
                    )}
                    <Plane tilt="flat" cap="Side by side" style={{ marginTop: 16 }}>
                      <ComparisonMatrix
                        model={matrix}
                        sort={sort}
                        onSort={(key) =>
                          setSort((prev) => ({ key, dir: prev.key === key && prev.dir === 'desc' ? 'asc' : 'desc' }))
                        }
                      />
                    </Plane>
                  </>
                );
              })()
            )
          ) : (
            <>
              {props.cohorts.length > 0 && (
                <label style={{ display: 'flex', alignItems: 'center', gap: 8, justifyContent: 'flex-end', fontSize: 12, color: 'var(--fg-muted)' }}>
                  Compared to
                  <select
                    aria-label="Compared to"
                    value={compareKey}
                    onChange={(e) => setCompareKey(e.target.value)}
                    className="hp-input hp-select"
                  >
                    {props.cohorts.map((c) => (
                      <option key={c.key} value={c.key}>
                        {c.label}
                      </option>
                    ))}
                    <option value="__all__">All {props.category}s</option>
                  </select>
                </label>
              )}
              {props.description && (
                <p className="hp-prose">{props.description}</p>
              )}
              {props.roleTags && props.roleTags.length > 0 && (
                <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap' }}>
                  {props.roleTags.map((t) => (
                    <BeamChip key={t}>{t}</BeamChip>
                  ))}
                </div>
              )}
              {visual.headline.length > 0 && (
                // UNCAPTIONED, deliberately. The flat version had no heading
                // here, and the page already has an "At a glance" pane above —
                // captioning this one the same put two identical headings on
                // one sheet. `Plane` supports an uncaptioned sheet for exactly
                // this: the figures are self-describing.
                <Plane tilt="flat" style={{ marginTop: 16 }}>
                  <HeadlineCallouts rows={visual.headline} />
                </Plane>
              )}
              {visual.radarAxes.length >= 3 && (
                <Plane tilt="flat" cap="Handling" style={{ marginTop: 16 }}>
                  <div className="hp-center">
                    <HandlingRadar axes={visual.radarAxes} />
                  </div>
                </Plane>
              )}
              {visual.groups.map((g) => (
                <Plane
                  key={g.title}
                  tilt="flat"
                  // A real heading, not a bare string: `Plane`'s cap is a span,
                  // and the visual view's groups are the sheet's structure.
                  cap={<h3>{g.title}</h3>}
                  style={{ marginTop: 16 }}
                >
                  {g.rows.map((r) => (
                    <StatBar key={r.label} row={r} />
                  ))}
                </Plane>
              ))}
            </>
          )}
        </>
      ) : (
        <DetailGroups groups={compact} />
      )}
    </>
  );
}

/**
 * The view / units / comparison toggles.
 *
 * REDRAWN. These were rounded 6px pills filled with `--accent-soft` when
 * active — a filled, rounded control, which the system does not have, in a
 * literal amber that ignored the calibration. They are lit hairline boxes now,
 * the same idiom as the Console's section tabs.
 *
 * `aria-pressed` is unchanged: these are toggle buttons, not links, because the
 * choice is persisted per reader rather than being addressable as a URL.
 */
function ViewToggle({
  view,
  units,
  onChange,
}: {
  view: 'visual' | 'compact';
  units: 'metric' | 'imperial';
  onChange: (p: Partial<KbPrefs>) => void;
}) {
  return (
    <div className="hp-kbtoggles">
      <div role="group" aria-label="View mode" className="hp-toggleset">
        <button
          type="button"
          aria-pressed={view === 'visual'}
          className="hp-toggle"
          data-active={view === 'visual' ? 'true' : undefined}
          onClick={() => onChange({ view: 'visual' })}
        >
          Visual
        </button>
        <button
          type="button"
          aria-pressed={view === 'compact'}
          className="hp-toggle"
          data-active={view === 'compact' ? 'true' : undefined}
          onClick={() => onChange({ view: 'compact' })}
        >
          Compact
        </button>
      </div>
      <span className="hp-togglediv" aria-hidden="true" />
      <div role="group" aria-label="Units" className="hp-toggleset">
        <button
          type="button"
          aria-pressed={units === 'metric'}
          className="hp-toggle"
          data-active={units === 'metric' ? 'true' : undefined}
          onClick={() => onChange({ units: 'metric' })}
        >
          Metric
        </button>
        <button
          type="button"
          aria-pressed={units === 'imperial'}
          className="hp-toggle"
          data-active={units === 'imperial' ? 'true' : undefined}
          onClick={() => onChange({ units: 'imperial' })}
        >
          Imperial
        </button>
      </div>
    </div>
  );
}
