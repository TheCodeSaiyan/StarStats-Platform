import React, { useMemo } from 'react';
import { SearchPicker } from './SearchPicker';
import { vocabularyFor } from '@/lib/kb-vocabulary';
import type { ReferenceCategory } from '@/lib/reference-types';

export interface CatalogItem {
  slug: string;
  display_name: string;
}

export interface SelectedEntry {
  slug: string;
  name: string;
  color: string;
  onRadar: boolean;
}

export interface ComparisonTrayProps {
  /** Drives every noun on this control. Without it the tray asked a reader
   *  browsing weapons to "Add ship…". */
  category: ReferenceCategory;
  anchorSlug: string;
  anchorName: string;
  selected: SelectedEntry[];
  catalog: CatalogItem[];
  max: number;
  onAdd: (slug: string) => void;
  onRemove: (slug: string) => void;
  onToggleRadar: (slug: string) => void;
  cohorts?: import('@/lib/reference-types').CohortRef[];
  onAddCohort?: (key: string) => void;
}

export function ComparisonTray(props: ComparisonTrayProps) {
  const vocab = vocabularyFor(props.category);
  const count = props.selected.length + 1; // + anchor
  const atCap = count >= props.max;

  const taken = useMemo(
    () => new Set([props.anchorSlug, ...props.selected.map((s) => s.slug)]),
    [props.anchorSlug, props.selected],
  );
  // Both pick-lists are fuzzy-ranked and portaled by `SearchPicker` — see
  // its header for why an in-card list was unusable here.
  const entryItems = useMemo(
    () =>
      props.catalog
        .filter((c) => !taken.has(c.slug))
        .map((c) => ({ key: c.slug, label: c.display_name })),
    [props.catalog, taken],
  );
  const cohortItems = useMemo(
    () => (props.cohorts ?? []).map((c) => ({ key: c.key, label: c.label, hint: c.kind })),
    [props.cohorts],
  );

  const chipStyle = (anchor: boolean): React.CSSProperties => ({
    display: 'inline-flex', alignItems: 'center', gap: 7, fontSize: 12,
    color: anchor ? 'var(--fg)' : 'var(--fg-muted)',
    // Hairline box, no fill, no radius — the system has no pill.
    background: 'transparent',
    border: `1px solid ${anchor ? 'var(--hot)' : 'rgba(var(--bR), var(--bG), var(--bB), 0.28)'}`,
    padding: '4px 10px',
  });

  return (
    <div className="ss-card" style={{ padding: '14px 16px' }}>
      <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
        {/* anchor chip (pinned) */}
        <span style={chipStyle(true)}>
          <span style={{ width: 10, height: 0, borderTop: '2px solid var(--hot)' }} />
          {props.anchorName}
          <span title="anchor (this page)" aria-label="anchor">⚓</span>
        </span>

        {props.selected.map((s) => (
          <span key={s.slug} style={chipStyle(false)}>
            <span style={{ width: 10, height: 0, borderTop: `2px solid ${s.color}` }} />
            {s.name}
            <button
              type="button"
              aria-label={`Toggle ${s.name} on radar`}
              onClick={() => props.onToggleRadar(s.slug)}
              style={{ background: 'none', border: 'none', cursor: 'pointer', color: s.onRadar ? 'var(--hot)' : 'var(--dim)', fontSize: 11 }}
            >
              ◎
            </button>
            <button
              type="button"
              aria-label={`Remove ${s.name}`}
              onClick={() => props.onRemove(s.slug)}
              style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--fg-muted)' }}
            >
              ✕
            </button>
          </span>
        ))}

        <SearchPicker
          label={`Add ${vocab.one} to comparison`}
          placeholder={atCap ? `Max ${props.max} reached` : `⌕ Add ${vocab.one}…`}
          disabled={atCap}
          items={entryItems}
          onPick={props.onAdd}
        />

        {cohortItems.length > 0 && props.onAddCohort && (
          <SearchPicker
            label="Add cohort to comparison"
            placeholder="+ Add cohort…"
            disabled={atCap}
            items={cohortItems}
            onPick={props.onAddCohort}
            browseWhenEmpty
            limit={12}
            minWidth={140}
          />
        )}

        <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--fg-muted)' }}>
          {count} / {props.max}
        </span>
      </div>
    </div>
  );
}
