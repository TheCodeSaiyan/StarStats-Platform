import React, { useMemo, useState } from 'react';

export interface CatalogItem {
  slug: string;
  display_name: string;
}

export interface SelectedShip {
  slug: string;
  name: string;
  color: string;
  onRadar: boolean;
}

export interface ComparisonTrayProps {
  anchorSlug: string;
  anchorName: string;
  selected: SelectedShip[];
  catalog: CatalogItem[];
  max: number;
  onAdd: (slug: string) => void;
  onRemove: (slug: string) => void;
  onToggleRadar: (slug: string) => void;
  cohorts?: import('@/lib/reference-types').CohortRef[];
  onAddCohort?: (key: string) => void;
}

export function ComparisonTray(props: ComparisonTrayProps) {
  const [query, setQuery] = useState('');
  const count = props.selected.length + 1; // + anchor
  const atCap = count >= props.max;

  const taken = useMemo(
    () => new Set([props.anchorSlug, ...props.selected.map((s) => s.slug)]),
    [props.anchorSlug, props.selected],
  );
  const suggestions = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [];
    return props.catalog
      .filter((c) => !taken.has(c.slug) && c.display_name.toLowerCase().includes(q))
      .slice(0, 8);
  }, [query, props.catalog, taken]);

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

        <div style={{ position: 'relative' }}>
          <input
            type="search"
            role="searchbox"
            aria-label="Add ship to comparison"
            placeholder={atCap ? `Max ${props.max} reached` : '⌕ Add ship…'}
            disabled={atCap}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => { if (e.key === 'Escape') setQuery(''); }}
            style={{
              fontSize: 12, padding: '6px 12px',
              background: 'transparent', color: 'var(--beam)',
              border: '1px dashed var(--border, rgba(255,255,255,.18))', minWidth: 160,
            }}
          />
          {suggestions.length > 0 && (
            <ul
              role="listbox"
              style={{
                position: 'absolute', top: '110%', left: 0, zIndex: 10, listStyle: 'none',
                margin: 0, padding: 4, minWidth: 200, maxHeight: 240, overflowY: 'auto',
                background: 'var(--void)',
                border: '1px solid rgba(var(--bR), var(--bG), var(--bB), 0.28)',
              }}
            >
              {suggestions.map((c) => (
                <li
                  key={c.slug}
                  role="option"
                  aria-selected={false}
                  tabIndex={0}
                  onClick={() => { props.onAdd(c.slug); setQuery(''); }}
                  onKeyDown={(e) => {
                    if (e.key === 'Enter' || e.key === ' ') {
                      e.preventDefault();
                      props.onAdd(c.slug);
                      setQuery('');
                    }
                  }}
                  style={{ color: 'var(--beam)', fontSize: 13, padding: '6px 8px', cursor: 'pointer' }}
                >
                  {c.display_name}
                </li>
              ))}
            </ul>
          )}
        </div>

        {props.cohorts && props.cohorts.length > 0 && props.onAddCohort && (
          <select
            aria-label="Add cohort to comparison"
            value=""
            disabled={atCap}
            onChange={(e) => {
              const v = e.target.value;
              if (v) props.onAddCohort!(v);
              e.target.value = '';
            }}
            style={{ fontSize: 12, padding: '6px 10px', background: 'transparent', color: 'var(--dim)', border: '1px dashed rgba(var(--bR), var(--bG), var(--bB), 0.28)' }}
          >
            <option value="">+ Add cohort…</option>
            {props.cohorts.map((c) => (
              <option key={c.key} value={c.key}>{c.label}</option>
            ))}
          </select>
        )}

        <span style={{ marginLeft: 'auto', fontSize: 11, color: 'var(--fg-muted)' }}>
          {count} / {props.max}
        </span>
      </div>
    </div>
  );
}
