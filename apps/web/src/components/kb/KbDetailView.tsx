'use client';

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

const SERIES_COLORS = ['#5BC8C0', '#9B8CF0', '#7Fd17F', '#E58FB0', '#E8C45B', '#6FA8E0', '#D98C6A', '#A0D060', '#C76FD0'];

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
              <button type="button" aria-pressed={showComparison} onClick={() => setShowComparison(true)} style={pillStyle(showComparison)}>Comparison</button>
              <button type="button" aria-pressed={!showComparison} onClick={() => setShowComparison(false)} style={pillStyle(!showComparison)}>Single</button>
            </div>
          )}

          {comparing && showComparison ? (
            vectors.length === 0 ? (
              <section className="ss-card" style={{ padding: '18px 20px' }}>
                <p style={{ color: 'var(--fg-muted)', fontSize: 13 }}>Loading comparison…</p>
              </section>
            ) : (
              (() => {
                const anchor = vectors.find((v) => v.slug === props.anchorSlug);
                const others = vectors.filter((v) => v.slug !== props.anchorSlug);
                if (!anchor) return null;
                const colorBySlug = new Map<string, string>([[anchor.slug, 'var(--accent, #E8A23C)']]);
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
                      <section className="ss-card" style={{ padding: '18px 20px' }}>
                        <ComparisonRadar
                          axisLabels={radarModel.axes.map((k) => RADAR_LABELS[k] ?? k)}
                          series={radarModel.series.map((s) => ({ ...s, color: colorBySlug.get(s.slug) ?? '#888888' }))}
                        />
                      </section>
                    )}
                    <section className="ss-card" style={{ padding: '18px 20px' }}>
                      <ComparisonMatrix
                        model={matrix}
                        sort={sort}
                        onSort={(key) =>
                          setSort((prev) => ({ key, dir: prev.key === key && prev.dir === 'desc' ? 'asc' : 'desc' }))
                        }
                      />
                    </section>
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
                    style={{
                      fontSize: 12, padding: '5px 10px', borderRadius: 6,
                      background: 'var(--surface, #16131d)', color: 'var(--fg)',
                      border: '1px solid var(--border, rgba(255,255,255,.12))',
                    }}
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
                <p style={{ fontSize: 15, lineHeight: 1.6, color: 'var(--fg-muted)' }}>{props.description}</p>
              )}
              {props.roleTags && props.roleTags.length > 0 && (
                <div style={{ display: 'flex', gap: 10, flexWrap: 'wrap' }}>
                  {props.roleTags.map((t) => (
                    <span key={t} style={{ background: 'var(--surface, #16131d)', border: '1px solid var(--border, rgba(255,255,255,.07))', borderRadius: 999, padding: '7px 14px', fontSize: 12, color: 'var(--fg-muted)' }}>{t}</span>
                  ))}
                </div>
              )}
              {visual.headline.length > 0 && <section className="ss-card" style={{ padding: '18px 20px' }}><HeadlineCallouts rows={visual.headline} /></section>}
              {visual.radarAxes.length >= 3 && (
                <section className="ss-card" style={{ padding: '18px 20px', display: 'flex', justifyContent: 'center' }}>
                  <HandlingRadar axes={visual.radarAxes} />
                </section>
              )}
              {visual.groups.map((g) => (
                <section key={g.title} className="ss-card" style={{ padding: '18px 20px' }}>
                  <h2 style={{ margin: '0 0 14px', fontSize: 14, fontWeight: 600 }}>{g.title}</h2>
                  {g.rows.map((r) => <StatBar key={r.label} row={r} />)}
                </section>
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

function pillStyle(active: boolean): React.CSSProperties {
  return {
    fontSize: 12, padding: '5px 12px', borderRadius: 6, cursor: 'pointer',
    border: '1px solid var(--border, rgba(255,255,255,.12))',
    background: active ? 'var(--accent-soft, rgba(232,162,60,0.14))' : 'transparent',
    color: active ? 'var(--accent, #E8A23C)' : 'var(--fg-muted)',
  };
}

function ViewToggle({ view, units, onChange }: { view: 'visual' | 'compact'; units: 'metric' | 'imperial'; onChange: (p: Partial<KbPrefs>) => void }) {
  const btn = (active: boolean): React.CSSProperties => ({
    fontSize: 12, padding: '5px 12px', borderRadius: 6, cursor: 'pointer',
    border: '1px solid var(--border, rgba(255,255,255,.1))',
    background: active ? 'var(--accent-soft, rgba(232,162,60,0.14))' : 'transparent',
    color: active ? 'var(--accent, #E8A23C)' : 'var(--fg-muted)',
  });
  return (
    <div style={{ display: 'flex', gap: 8, alignItems: 'center', justifyContent: 'flex-end' }}>
      <div role="group" aria-label="View mode" style={{ display: 'flex', gap: 8 }}>
        <button type="button" aria-pressed={view === 'visual'} style={btn(view === 'visual')} onClick={() => onChange({ view: 'visual' })}>Visual</button>
        <button type="button" aria-pressed={view === 'compact'} style={btn(view === 'compact')} onClick={() => onChange({ view: 'compact' })}>Compact</button>
      </div>
      <span style={{ width: 1, height: 18, background: 'var(--border, rgba(255,255,255,.1))', margin: '0 4px' }} />
      <div role="group" aria-label="Units" style={{ display: 'flex', gap: 8 }}>
        <button type="button" aria-pressed={units === 'metric'} style={btn(units === 'metric')} onClick={() => onChange({ units: 'metric' })}>Metric</button>
        <button type="button" aria-pressed={units === 'imperial'} style={btn(units === 'imperial')} onClick={() => onChange({ units: 'imperial' })}>Imperial</button>
      </div>
    </div>
  );
}
