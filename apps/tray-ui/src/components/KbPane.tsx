import { useEffect, useMemo, useState } from 'react';
import { open as openShell } from '@tauri-apps/plugin-shell';
import { TrayCard } from './tray/primitives';
import { friendlyError } from '../lib/friendlyError';
import {
  EMPTY_ALL_BUNDLES,
  REFERENCE_CATEGORIES,
  loadAllReferenceBundles,
  webKbUrl,
  type AllReferenceBundles,
  type ReferenceCategory,
  type ReferenceEntry,
} from '../lib/reference';

/**
 * Tray-side Knowledge base pane. Browse the four synced catalogues
 * (vehicle / weapon / item / location). Search narrows in-memory;
 * a click on an entry opens its detail page on the StarStats web
 * app via the OS default browser (`@tauri-apps/plugin-shell`) —
 * the tray window is too compact for a useful detail view of its
 * own, so we re-use the work we already shipped on web.
 *
 * Fetch contract: depends on `apiUrl` (paired API server) — when
 * the tray isn't paired we render a guidance message instead of
 * trying to hit a phantom server. `webOrigin` is provided by the
 * shared Config; same place the LeftRail-equivalents read it from.
 */

interface Props {
  apiUrl: string | null;
  webOrigin: string | null;
}

const CATEGORY_LABELS: Record<ReferenceCategory, string> = {
  vehicle: 'Vehicles',
  weapon: 'Weapons',
  item: 'Items',
  location: 'Locations',
};

const PAGE_SIZE = 50;

export function KbPane({ apiUrl, webOrigin }: Props) {
  const [bundles, setBundles] = useState<AllReferenceBundles>(EMPTY_ALL_BUNDLES);
  const [active, setActive] = useState<ReferenceCategory>('vehicle');
  const [q, setQ] = useState('');
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // Fetch all four bundles once on mount (and when apiUrl changes).
  // The /v1/reference endpoints are cheap server-side (single
  // GROUP BY) and cached at the fetch layer; one cold load per
  // session is the steady state.
  useEffect(() => {
    if (!apiUrl) {
      setBundles(EMPTY_ALL_BUNDLES);
      return;
    }
    let cancelled = false;
    setLoading(true);
    setError(null);
    loadAllReferenceBundles(apiUrl)
      .then((b) => {
        if (cancelled) return;
        setBundles(b);
      })
      .catch((e: unknown) => {
        if (cancelled) return;
        const f = friendlyError(e);
        setError(`${f.title}: ${f.body}`);
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [apiUrl]);

  const activeBundle = bundles[active];

  // In-memory search + sort. `display_name` already sorted A→Z at
  // the fetcher; substring filter doesn't change order.
  const filtered = useMemo(() => {
    if (!q.trim()) return activeBundle.list;
    const needle = q.trim().toLowerCase();
    return activeBundle.list.filter((e) => {
      return (
        e.display_name.toLowerCase().includes(needle) ||
        e.class_name.toLowerCase().includes(needle)
      );
    });
  }, [activeBundle, q]);

  // Reset search and rewind to top when switching categories — the
  // current filter probably doesn't make sense in the next set.
  const pickCategory = (next: ReferenceCategory) => {
    setActive(next);
    setQ('');
  };

  const handleOpen = (entry: ReferenceEntry) => {
    if (!webOrigin) return;
    const url = webKbUrl(webOrigin, active, entry);
    void openShell(url);
  };

  if (!apiUrl) {
    return (
      <TrayCard title="Knowledge base">
        <p
          style={{
            margin: 0,
            color: 'var(--fg-muted)',
            fontSize: 13,
            lineHeight: 1.55,
          }}
        >
          Pair this tray with a StarStats server (Calibrate → Remote
          sync) to browse the synced catalogue of ships, weapons,
          items, and locations.
        </p>
      </TrayCard>
    );
  }

  return (
    <TrayCard
      title="Knowledge base"
      kicker={
        loading
          ? 'loading…'
          : `${filtered.length.toLocaleString()} ${active}${filtered.length === 1 ? '' : 's'}`
      }
    >
      <div
        role="tablist"
        aria-label="Category"
        style={{ display: 'flex', gap: 4, flexWrap: 'wrap', marginBottom: 10 }}
      >
        {REFERENCE_CATEGORIES.map((c) => {
          const selected = c === active;
          const count = bundles[c].list.length;
          return (
            <button
              key={c}
              type="button"
              role="tab"
              aria-selected={selected}
              onClick={() => pickCategory(c)}
              style={{
                padding: '4px 10px',
                fontSize: 11,
                borderRadius: 'var(--r-sm)',
                border: `1px solid ${selected ? 'var(--accent)' : 'var(--border)'}`,
                background: selected ? 'var(--accent-soft)' : 'transparent',
                color: 'var(--fg)',
                cursor: 'pointer',
                letterSpacing: '0.04em',
              }}
            >
              {CATEGORY_LABELS[c]}
              {count > 0 && (
                <span style={{ color: 'var(--fg-dim)', marginLeft: 6 }}>
                  {count.toLocaleString()}
                </span>
              )}
            </button>
          );
        })}
      </div>

      <input
        type="search"
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder="Search class name or display name…"
        autoComplete="off"
        spellCheck={false}
        style={{
          width: '100%',
          padding: '7px 10px',
          background: 'var(--bg)',
          border: '1px solid var(--border)',
          borderRadius: 'var(--r-sm)',
          color: 'var(--fg)',
          fontSize: 12,
          marginBottom: 10,
          boxSizing: 'border-box',
        }}
      />

      {error && (
        <p style={{ margin: '0 0 8px', fontSize: 12, color: 'var(--danger)' }}>
          {error}
        </p>
      )}

      <ul
        style={{
          listStyle: 'none',
          margin: 0,
          padding: 0,
          display: 'flex',
          flexDirection: 'column',
          gap: 4,
          maxHeight: 420,
          overflowY: 'auto',
        }}
      >
        {filtered.slice(0, PAGE_SIZE).map((entry) => {
          const canOpen = Boolean(webOrigin && entry.slug);
          return (
            <li
              key={entry.class_name}
              style={{
                display: 'flex',
                flexDirection: 'column',
                gap: 2,
                padding: '6px 8px',
                background: 'var(--surface-2)',
                border: '1px solid var(--border)',
                borderRadius: 'var(--r-sm)',
                cursor: canOpen ? 'pointer' : 'default',
              }}
              onClick={canOpen ? () => handleOpen(entry) : undefined}
              role={canOpen ? 'button' : undefined}
              tabIndex={canOpen ? 0 : undefined}
              onKeyDown={
                canOpen
                  ? (ev) => {
                      if (ev.key === 'Enter' || ev.key === ' ') {
                        ev.preventDefault();
                        handleOpen(entry);
                      }
                    }
                  : undefined
              }
            >
              <span style={{ fontSize: 13, color: 'var(--fg)' }}>
                {entry.display_name}
                {!entry.slug && (
                  <span
                    style={{
                      marginLeft: 6,
                      color: 'var(--fg-dim)',
                      fontSize: 10,
                    }}
                  >
                    (no slug — open browser to browse this category)
                  </span>
                )}
              </span>
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 10,
                  color: 'var(--fg-dim)',
                  wordBreak: 'break-word',
                }}
              >
                {entry.class_name}
              </span>
              {renderInlineSummary(entry)}
            </li>
          );
        })}
        {filtered.length === 0 && !loading && (
          <li style={{ fontSize: 12, color: 'var(--fg-dim)', padding: '6px 0' }}>
            {q.trim()
              ? 'No entries match this search.'
              : 'Catalogue is empty — has the server-side sync run yet?'}
          </li>
        )}
        {filtered.length > PAGE_SIZE && (
          <li
            style={{
              fontSize: 11,
              color: 'var(--fg-dim)',
              padding: '6px 0',
              textAlign: 'center',
            }}
          >
            Showing first {PAGE_SIZE} of {filtered.length}. Refine your
            search to see more.
          </li>
        )}
      </ul>
    </TrayCard>
  );
}

/** Render the first 2-3 populated summary fields as a one-line
 *  hint underneath the class name. Keeps the row compact while
 *  giving the user enough context to spot the right entry without
 *  opening the browser. Discriminates on summary.category so each
 *  branch picks the right curated fields. */
function renderInlineSummary(entry: ReferenceEntry) {
  const parts: string[] = [];
  const push = (label: string, value: string | undefined) => {
    if (value && value.length > 0 && parts.length < 3) {
      parts.push(`${label}: ${value}`);
    }
  };
  const s = entry.summary;
  switch (s.category) {
    case 'vehicle':
      push('manufacturer', s.manufacturer);
      push('role', s.role);
      push('size', s.hull_size);
      break;
    case 'weapon':
      push('manufacturer', s.manufacturer);
      push('size', s.size);
      push('damage', s.damage_type);
      break;
    case 'item':
      push('manufacturer', s.manufacturer);
      push('type', s.item_type);
      push('grade', s.grade);
      break;
    case 'location':
      push('system', s.system);
      push('parent', s.parent);
      push('type', s.classification);
      break;
  }
  if (parts.length === 0) return null;
  return (
    <span
      style={{
        fontSize: 10,
        color: 'var(--fg-muted)',
        marginTop: 2,
      }}
    >
      {parts.join(' · ')}
    </span>
  );
}
