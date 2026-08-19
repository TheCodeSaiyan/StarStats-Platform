/**
 * Tray UI — Logs view: browse the locally-stored events on disk.
 *
 *
 * Data path is server-side. Search + filter + pagination are all
 * driven by `api.searchEvents`, which runs a case-insensitive
 * substring match against `type` and the parsed `payload` JSON,
 * narrows by exact `type_filter` when a type pill is active, and
 * pages via a `before_id` cursor. Page size is `LOGS_PAGE_LIMIT`
 * (200), well under the Rust-side IPC cap (`MAX_TIMELINE_LIMIT`
 * = 5000). The query input debounces to 250ms before firing a
 * fresh search; "Load more" appends to the page accumulator.
 *
 * Storage-level stats (`api.getStorageStats` + `api.countQuarantined`)
 * still tick on a 10s interval — these are cheap aggregates and
 * independent of the search predicate. The search result set is
 * NOT refreshed on the tick, to avoid jerking the list mid-browse.
 *
 * The detail drawer is portaled to `document.body` via
 * `react-dom`'s `createPortal`. The pane is wrapped at App-level
 * in `<div className="ss-screen-enter">`, which carries an
 * animated `transform` and therefore creates a containing block
 * for `position: fixed` descendants — without the portal, the
 * scrim+drawer would pin to that wrapper instead of the viewport
 * and slide out of view when the list scrolls.
 */

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type ReactNode,
} from 'react';
import { createPortal } from 'react-dom';
import {
  api,
  type StorageStats,
  type TimelineEntry,
} from '../api';
import {
  GhostButton,
  StatPill,
  StatusDot,
  TrayCard,
  KV,
  type Tone,
} from './tray/primitives';
import { humanTitleForEntryReact } from './tray/format-react';
import { friendlyError } from '../lib/friendlyError';
import {
  ageLabel,
  fmtBytes,
  fmtDate,
  fmtTime,
  humanTitleForEntry,
  toneForType,
  TONE_VAR,
} from './tray/format';
import { TransactionsCard } from './TransactionsCard';

const REFRESH_MS = 10_000;

/// Page size for the server-side `search_events` query. Each "Load
/// more" tap pulls one more page; the smaller page (vs the old
/// 1000-row window) keeps the initial paint fast and lets a casual
/// browse stay scoped to the most recent events without dragging in
/// rows the user will never look at.
const LOGS_PAGE_LIMIT = 200;

/// Debounce window for the search input. Keeps every keystroke from
/// firing a SQL query, but short enough that the result feels live.
const SEARCH_DEBOUNCE_MS = 250;

interface DayGroup<T> {
  /** Epoch ms of the local midnight — a year-unique React key. The
   *  `label` omits the year ("Mon, Jul 8"), so two same-day groups from
   *  different years would collide if keyed by label (L9). */
  key: number;
  label: string;
  items: T[];
}

/** Bucket events by their local-day timestamp. Returns groups in
 * insertion order, which (because the timeline is newest-first) means
 * "Today" → "Yesterday" → older days. */
function groupByDay<T extends { timestamp: string }>(
  events: T[],
): DayGroup<T>[] {
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const todayKey = today.getTime();

  const groups = new Map<number, DayGroup<T>>();
  for (const e of events) {
    const d = new Date(e.timestamp);
    if (Number.isNaN(d.getTime())) continue;
    d.setHours(0, 0, 0, 0);
    const key = d.getTime();
    let label: string;
    if (key === todayKey) label = 'Today';
    else if (key === todayKey - 86_400_000) label = 'Yesterday';
    else
      label = d.toLocaleDateString(undefined, {
        weekday: 'short',
        month: 'short',
        day: 'numeric',
      });
    let bucket = groups.get(key);
    if (!bucket) {
      bucket = { key, label, items: [] };
      groups.set(key, bucket);
    }
    bucket.items.push(e);
  }
  return [...groups.values()];
}

interface TypePillProps {
  label: string;
  /// Optional now that filtering is server-side. The per-type count
  /// is the "how many rows of this type exist in the whole store"
  /// answer — we don't have that cheaply with pagination (the loaded
  /// pages are post-filter, so they can't reveal sibling-type totals),
  /// so we omit the count rather than show a misleading window count.
  /// The active pill renders the search-result `total` instead.
  count?: number;
  active: boolean;
  onClick: () => void;
}

function TypePill({ label, count, active, onClick }: TypePillProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        background: active ? 'var(--accent-soft)' : 'var(--surface)',
        color: active ? 'var(--accent)' : 'var(--fg-muted)',
        border: `1px solid ${active ? 'var(--accent)' : 'var(--border)'}`,
        borderRadius: 'var(--r-pill)',
        padding: '3px 9px',
        fontFamily: 'var(--font-mono)',
        fontSize: 11,
        cursor: 'pointer',
        whiteSpace: 'nowrap',
        transition: 'all 120ms',
      }}
    >
      <span>{label}</span>
      {count !== undefined && (
        <span
          style={{
            fontSize: 10,
            color: active ? 'var(--accent)' : 'var(--fg-dim)',
            fontVariantNumeric: 'tabular-nums',
          }}
        >
          {count}
        </span>
      )}
    </button>
  );
}

const SEARCH_BAR_STYLE: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 8,
  background: 'var(--surface)',
  border: '1px solid var(--border)',
  borderRadius: 'var(--r-md)',
  padding: '6px 10px',
};

const SEARCH_INPUT_STYLE: CSSProperties = {
  flex: 1,
  background: 'transparent',
  border: 'none',
  color: 'var(--fg)',
  fontFamily: 'var(--font-mono)',
  fontSize: 12,
  padding: '4px 0',
};

const SCRIM_STYLE: CSSProperties = {
  position: 'fixed',
  inset: 0,
  background: 'rgba(0, 0, 0, 0.55)',
  zIndex: 100,
  display: 'flex',
  alignItems: 'flex-end',
  justifyContent: 'center',
  animation: 'fadeIn 180ms',
};

const DRAWER_STYLE: CSSProperties = {
  width: '100%',
  maxWidth: 720,
  background: 'var(--bg-elev)',
  borderTop: '1px solid var(--border-strong)',
  borderTopLeftRadius: 'var(--r-lg)',
  borderTopRightRadius: 'var(--r-lg)',
  padding: 20,
  maxHeight: '70vh',
  overflowY: 'auto',
  animation: 'slideUp 240ms var(--ease-out)',
};

interface LogsPaneProps {
  /** Catalog-driven class-name prettifier. Passed through to every
   *  `humanTitleForEntry` call so AEGS_Avenger_Stalker renders as
   *  "Aegis Avenger Stalker" in the search row. Empty / undefined →
   *  raw Rust-formatted summary string (still readable). */
  prettyLookup?: import('./tray/format').PrettyLookup;
  /** Full reference bundles. When supplied alongside `webOrigin`,
   *  search-row summaries swap the plain-text prettifier for a
   *  ReactNode variant that wraps class-name tokens in
   *  `<TrayEntityLink>`, opening the KB page in the user's default
   *  browser on click. */
  bundles?: import('../lib/reference').AllReferenceBundles;
  /** Paired API's companion web origin. Required for KB links to
   *  resolve. */
  webOrigin?: string | null;
}

export function LogsPane({
  prettyLookup,
  bundles,
  webOrigin,
}: LogsPaneProps = {}) {
  // Accumulator for paginated results. Each page from `search_events`
  // is appended (Load more) or replaces the list (new search). The
  // selected-row drawer reads from this same accumulator, so a row
  // loaded via "Load more" stays openable after subsequent pages.
  const [pages, setPages] = useState<TimelineEntry[]>([]);
  // Server-reported total for the active filter. Null = first fetch
  // hasn't completed yet (drives the initial loading shimmer).
  const [total, setTotal] = useState<number | null>(null);
  // Server-reported pagination flag — drives the "Load more" button.
  const [hasMore, setHasMore] = useState(false);
  // True while a fresh search (debouncedQuery / activeType change) is
  // in flight. Distinct from `loadingMore` so the existing list
  // doesn't blank out when the user pages.
  const [loading, setLoading] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);
  const [stats, setStats] = useState<StorageStats | null>(null);
  const [query, setQuery] = useState('');
  const [debouncedQuery, setDebouncedQuery] = useState('');
  const [activeType, setActiveType] = useState<string>('all');
  const [selectedId, setSelectedId] = useState<number | null>(null);
  const [showRaw, setShowRaw] = useState(false);
  const [copyState, setCopyState] = useState<'idle' | 'copied' | 'failed'>(
    'idle',
  );
  const [noiseError, setNoiseError] = useState<string | null>(null);
  const [pendingNoise, setPendingNoise] = useState(false);
  const [retryState, setRetryState] = useState<'idle' | 'kicked' | 'failed'>(
    'idle',
  );
  const [retryPending, setRetryPending] = useState(false);
  // Persistent count of quarantined rows in storage. Null = not yet
  // fetched / fetch failed; treated as "show nothing" by the UI.
  const [quarantinedCount, setQuarantinedCount] = useState<number | null>(null);
  const [releasePending, setReleasePending] = useState(false);
  const [releaseState, setReleaseState] = useState<
    | { kind: 'idle' }
    | { kind: 'released'; count: number }
    | { kind: 'failed'; message: string }
  >({ kind: 'idle' });

  // Refresh storage-level counters (DB size, quarantined count). These
  // are independent of the search filter so they tick on their own
  // 10s cadence — refetching them never disturbs an in-flight browse.
  const refreshStats = useCallback(
    async (signal?: { aborted: boolean }): Promise<void> => {
      try {
        const [st, qc] = await Promise.allSettled([
          api.getStorageStats(),
          api.countQuarantined(),
        ]);
        if (signal?.aborted) return;
        if (st.status === 'fulfilled') setStats(st.value);
        if (qc.status === 'fulfilled') setQuarantinedCount(qc.value);
      } catch (err) {
        // eslint-disable-next-line no-console
        console.warn('Failed to refresh storage stats', err);
      }
    },
    [],
  );

  useEffect(() => {
    const signal = { aborted: false };
    void refreshStats(signal);
    const handle = window.setInterval(() => {
      void refreshStats(signal);
    }, REFRESH_MS);
    return () => {
      signal.aborted = true;
      window.clearInterval(handle);
    };
  }, [refreshStats]);

  // Debounce typing in the search input. Each keystroke would
  // otherwise re-hit `search_events`; coalescing into 250ms windows
  // keeps the DB cool without hurting the live-feel.
  useEffect(() => {
    const handle = window.setTimeout(() => {
      setDebouncedQuery(query);
    }, SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
  }, [query]);

  // Bumped every time the filter scope changes, so an in-flight
  // "Load more" from the previous scope can detect that it's stale and
  // discard its result instead of appending old-filter rows (M-U4).
  const searchGeneration = useRef(0);

  // Run a fresh search whenever the debounced query or active type
  // changes. The result REPLACES the accumulator (this is a new
  // filter scope, so any previously-loaded pages no longer apply).
  useEffect(() => {
    searchGeneration.current += 1;
    const signal = { aborted: false };
    setLoading(true);
    (async () => {
      try {
        const result = await api.searchEvents({
          query: debouncedQuery || undefined,
          type_filter: activeType === 'all' ? undefined : activeType,
          limit: LOGS_PAGE_LIMIT,
        });
        if (signal.aborted) return;
        setPages(result.entries);
        setTotal(result.total);
        setHasMore(result.has_more);
      } catch (err) {
        if (signal.aborted) return;
        // eslint-disable-next-line no-console
        console.warn('Failed to search events', err);
      } finally {
        if (!signal.aborted) setLoading(false);
      }
    })();
    return () => {
      signal.aborted = true;
    };
  }, [debouncedQuery, activeType]);

  // Pull the next page (rows older than the smallest currently-loaded
  // id). Appends to the accumulator. Concurrent calls are no-ops via
  // the `loadingMore` / `hasMore` guards on the button.
  const handleLoadMore = useCallback(async () => {
    if (pages.length === 0 || !hasMore || loadingMore) return;
    setLoadingMore(true);
    const generationAtStart = searchGeneration.current;
    try {
      const beforeId = pages.reduce(
        (min, e) => (e.id < min ? e.id : min),
        pages[0].id,
      );
      const result = await api.searchEvents({
        query: debouncedQuery || undefined,
        type_filter: activeType === 'all' ? undefined : activeType,
        before_id: beforeId,
        limit: LOGS_PAGE_LIMIT,
      });
      // If the filter scope changed while this page was in flight, a fresh
      // search has already replaced `pages` — discard these stale rows
      // rather than appending them / clobbering total/hasMore (M-U4).
      if (searchGeneration.current !== generationAtStart) return;
      setPages((prev) => [...prev, ...result.entries]);
      setTotal(result.total);
      setHasMore(result.has_more);
    } catch (err) {
      // eslint-disable-next-line no-console
      console.warn('Failed to load more events', err);
    } finally {
      setLoadingMore(false);
    }
  }, [pages, hasMore, loadingMore, debouncedQuery, activeType]);

  // Manual re-run of the active search. Used after mark-as-noise so
  // the newly-suppressed event vanishes from the list without waiting
  // for the user to retype.
  const refreshSearch = useCallback(async () => {
    try {
      const result = await api.searchEvents({
        query: debouncedQuery || undefined,
        type_filter: activeType === 'all' ? undefined : activeType,
        limit: LOGS_PAGE_LIMIT,
      });
      setPages(result.entries);
      setTotal(result.total);
      setHasMore(result.has_more);
    } catch (err) {
      // eslint-disable-next-line no-console
      console.warn('Failed to refresh search', err);
    }
  }, [debouncedQuery, activeType]);

  // Esc-to-close for the detail drawer. We bind the listener only
  // while a row is selected so we don't intercept Escape elsewhere.
  useEffect(() => {
    if (selectedId === null) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') setSelectedId(null);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [selectedId]);

  // Reset transient drawer state when a different row is selected
  // (or the drawer is closed).
  useEffect(() => {
    setShowRaw(false);
    setCopyState('idle');
    setNoiseError(null);
    setRetryState('idle');
  }, [selectedId]);

  // `pages` IS the filtered result now — server-side. We still compute
  // synced/pending against the loaded window because per-row sync
  // state isn't surfaced as a separate counter; this matches the
  // original "filtered window" semantic.
  const loadedCount = pages.length;
  const unsyncedCount = pages.filter((e) => !e.synced).length;
  const syncedCount = loadedCount - unsyncedCount;

  // Distinct event types observed across the loaded pages. We can't
  // get true per-type counts cheaply (would require N more queries),
  // so the pills carry no count badge — see TypePillProps.count
  // comment for the rationale. The order is "All" first, then the
  // observed types alphabetically so they don't reshuffle as new
  // pages load.
  const allTypes = useMemo<string[]>(() => {
    const s = new Set<string>();
    for (const e of pages) s.add(e.event_type);
    return [...s].sort();
  }, [pages]);

  const grouped = useMemo(() => groupByDay(pages), [pages]);

  // Memoise each row's rendered title so the O(catalog × rows) entity-link
  // scan (findDisplayNameHits walks all four catalogues per row) reruns only
  // when the rows or the reference data change — not on every keystroke,
  // selection, or 10s stats tick (M-U6). Keyed by row id. prettyLookup is
  // memoised in App so its identity stays stable across those re-renders.
  const titleByRowId = useMemo(() => {
    const m = new Map<number, ReactNode>();
    for (const e of pages) {
      m.set(
        e.id,
        bundles && webOrigin
          ? humanTitleForEntryReact(e, bundles, webOrigin)
          : humanTitleForEntry(e, prettyLookup),
      );
    }
    return m;
  }, [pages, bundles, webOrigin, prettyLookup]);

  const selected = useMemo(
    () => pages.find((e) => e.id === selectedId) ?? null,
    [pages, selectedId],
  );

  // Stored / synced/ pending pills — we prefer the storage_stats
  // total when available since it counts the full table, not just
  // the most-recent page we render. Synced/Pending are still
  // window-scoped because they require the per-row id comparison
  // and we don't carry that across the full table yet.
  const storedDisplay = stats
    ? stats.total_events.toLocaleString()
    : loadedCount.toLocaleString();
  const dbSizeDisplay = stats ? fmtBytes(stats.db_size_bytes) : '—';

  const pendingTone: Tone = unsyncedCount > 0 ? 'warn' : 'default';

  const handleCopyRaw = async () => {
    if (!selected) return;
    try {
      await navigator.clipboard.writeText(selected.raw_line);
      setCopyState('copied');
      window.setTimeout(() => setCopyState('idle'), 1500);
    } catch (err) {
      // eslint-disable-next-line no-console
      console.warn('Failed to copy raw line', err);
      setCopyState('failed');
      window.setTimeout(() => setCopyState('idle'), 1500);
    }
  };

  const handleMarkAsNoise = async () => {
    if (!selected) return;
    setPendingNoise(true);
    setNoiseError(null);
    try {
      await api.markEventAsNoise(selected.event_type);
      setSelectedId(null);
      await Promise.all([refreshSearch(), refreshStats()]);
    } catch (err) {
      // Tauri invoke rejections are plain strings, so `instanceof Error` is
      // always false — friendlyError handles strings (and maps common
      // failures to readable copy). See M-U3.
      const message = friendlyError(err).body;
      setNoiseError(message);
    } finally {
      setPendingNoise(false);
    }
  };

  const handleReleaseQuarantined = async () => {
    setReleasePending(true);
    setReleaseState({ kind: 'idle' });
    try {
      const count = await api.releaseQuarantined();
      setReleaseState({ kind: 'released', count });
      // Refresh the stats so the "Quarantined" pill drops to 0
      // immediately rather than waiting for the next 10s tick.
      void refreshStats();
      // Auto-clear the success banner after a few seconds.
      window.setTimeout(() => setReleaseState({ kind: 'idle' }), 4000);
    } catch (err) {
      // Tauri invoke rejections are plain strings, so `instanceof Error` is
      // always false — friendlyError handles strings (and maps common
      // failures to readable copy). See M-U3.
      const message = friendlyError(err).body;
      setReleaseState({ kind: 'failed', message });
    } finally {
      setReleasePending(false);
    }
  };

  const handleRetrySync = async () => {
    setRetryPending(true);
    try {
      await api.retrySyncNow();
      setRetryState('kicked');
      // Give the worker a beat to drain before refetching, otherwise
      // the user clicks Retry and sees the same Pending count for a
      // moment because we lapped the worker's loop.
      window.setTimeout(() => {
        void Promise.all([refreshSearch(), refreshStats()]);
      }, 800);
      window.setTimeout(() => setRetryState('idle'), 2500);
    } catch (err) {
      // eslint-disable-next-line no-console
      console.warn('Failed to kick sync worker', err);
      setRetryState('failed');
      window.setTimeout(() => setRetryState('idle'), 2500);
    } finally {
      setRetryPending(false);
    }
  };

  const showLoadingState = loading && pages.length === 0;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      {/* HEADLINE STAT STRIP */}
      <div style={{ display: 'flex', gap: 8, flexWrap: 'wrap' }}>
        <StatPill label="Stored" value={storedDisplay} tone="accent" />
        <StatPill
          label="Synced"
          value={syncedCount.toLocaleString()}
          tone="ok"
        />
        <StatPill
          label="Pending"
          value={unsyncedCount.toLocaleString()}
          tone={pendingTone}
        />
        {/* Quarantined pill only renders when there are rows shelved.
            Hidden at zero so the strip stays quiet in the happy path. */}
        {quarantinedCount !== null && quarantinedCount > 0 && (
          <StatPill
            label="Quarantined"
            value={quarantinedCount.toLocaleString()}
            tone="danger"
          />
        )}
        <StatPill label="DB size" value={dbSizeDisplay} />
      </div>

      {/* QUARANTINE RECOVERY ROW
          Only shown when the storage has poison-pill-shelved rows. The
          release flips `sent_at` back to NULL on every `__quarantined_*`
          row and kicks the sync worker; if the underlying server-side
          cause persists, the bisector will re-quarantine on the next
          drain (capped per-drain), so calling this repeatedly without
          fixing root cause won't make progress. */}
      {quarantinedCount !== null && quarantinedCount > 0 && (
        <div
          role="status"
          aria-live="polite"
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: 10,
            padding: '8px 12px',
            background: 'var(--surface)',
            border: '1px solid var(--border)',
            borderLeft: '3px solid var(--danger)',
            borderRadius: 'var(--r-sm)',
            fontSize: 12,
          }}
        >
          <span style={{ color: 'var(--fg)', flex: 1 }}>
            {quarantinedCount.toLocaleString()} event
            {quarantinedCount === 1 ? '' : 's'} quarantined by the sync
            worker. Release to retry on the next drain.
          </span>
          {releaseState.kind === 'released' && (
            <span
              style={{
                color: 'var(--ok)',
                fontFamily: 'var(--font-mono)',
                fontSize: 11,
              }}
            >
              Released {releaseState.count.toLocaleString()}
            </span>
          )}
          {releaseState.kind === 'failed' && (
            <span
              style={{
                color: 'var(--danger)',
                fontSize: 11,
              }}
              title={releaseState.message}
            >
              Release failed
            </span>
          )}
          <GhostButton
            type="button"
            onClick={handleReleaseQuarantined}
            disabled={releasePending}
            title="Flip sent_at back to NULL and kick the sync worker"
          >
            {releasePending ? 'Releasing…' : 'Release'}
          </GhostButton>
        </div>
      )}

      {/* SEARCH BAR */}
      <div style={SEARCH_BAR_STYLE}>
        <span
          style={{
            color: 'var(--fg-dim)',
            fontSize: 13,
            fontFamily: 'var(--font-mono)',
          }}
        >
          ⌕
        </span>
        <input
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder="Filter by type or summary…"
          style={SEARCH_INPUT_STYLE}
        />
        {query && (
          <button
            type="button"
            onClick={() => setQuery('')}
            style={{
              background: 'transparent',
              border: 'none',
              color: 'var(--fg-dim)',
              cursor: 'pointer',
              fontSize: 11,
              padding: 0,
              fontFamily: 'inherit',
            }}
          >
            clear
          </button>
        )}
        <span
          style={{
            color: 'var(--fg-dim)',
            fontSize: 11,
            fontFamily: 'var(--font-mono)',
            paddingLeft: 8,
            borderLeft: '1px solid var(--border)',
          }}
        >
          {loadedCount.toLocaleString()} / {total !== null ? total.toLocaleString() : '…'}
        </span>
      </div>

      {/* TYPE PILL ROW
          Pills no longer carry per-type counts — pagination makes the
          "true" count unknowable without an extra round-trip per type.
          See TypePillProps.count for the rationale. */}
      {allTypes.length > 0 && (
        <div
          style={{
            display: 'flex',
            flexWrap: 'wrap',
            gap: 4,
            marginTop: -4,
          }}
        >
          <TypePill
            label="All"
            active={activeType === 'all'}
            onClick={() => setActiveType('all')}
          />
          {allTypes.map((type) => (
            <TypePill
              key={type}
              label={type}
              active={activeType === type}
              onClick={() => setActiveType(type)}
            />
          ))}
        </div>
      )}

      <TransactionsCard />

      {/* GROUPED LIST */}
      {showLoadingState ? (
        <TrayCard>
          <p
            style={{
              margin: 0,
              color: 'var(--fg-dim)',
              fontSize: 13,
              textAlign: 'center',
              padding: '12px 0',
            }}
          >
            Loading events…
          </p>
        </TrayCard>
      ) : grouped.length === 0 ? (
        <TrayCard>
          <p
            style={{
              margin: 0,
              color: 'var(--fg-dim)',
              fontSize: 13,
              textAlign: 'center',
              padding: '12px 0',
            }}
          >
            Scope is clear. No events match this filter.
          </p>
        </TrayCard>
      ) : (
        grouped.map((g) => (
          <TrayCard
            key={g.key}
            title={g.label}
            kicker={`${g.items.length} events`}
          >
            <ol
              style={{
                listStyle: 'none',
                margin: 0,
                padding: 0,
                display: 'flex',
                flexDirection: 'column',
                gap: 1,
              }}
            >
              {g.items.map((e) => {
                const tone = toneForType(e.event_type);
                const isSelected = selectedId === e.id;
                return (
                  <li
                    key={e.id}
                    role="button"
                    tabIndex={0}
                    aria-pressed={isSelected}
                    onClick={() => setSelectedId(e.id)}
                    onKeyDown={(ev) => {
                      // H7: rows were mouse-only. Make them keyboard-
                      // operable — Enter/Space opens the detail drawer.
                      if (ev.key === 'Enter' || ev.key === ' ') {
                        ev.preventDefault();
                        setSelectedId(e.id);
                      }
                    }}
                    style={{
                      display: 'grid',
                      gridTemplateColumns: '60px 1fr auto auto',
                      gap: 10,
                      alignItems: 'baseline',
                      padding: '5px 8px',
                      borderLeft: `2px solid ${TONE_VAR[tone]}`,
                      fontSize: 12,
                      cursor: 'pointer',
                      background: isSelected
                        ? 'var(--surface-2)'
                        : 'transparent',
                      transition: 'background 120ms',
                    }}
                    onMouseEnter={(ev) => {
                      if (!isSelected)
                        ev.currentTarget.style.background = 'var(--surface-2)';
                    }}
                    onMouseLeave={(ev) => {
                      if (!isSelected)
                        ev.currentTarget.style.background = 'transparent';
                    }}
                  >
                    <span
                      style={{
                        color: 'var(--fg-dim)',
                        fontFamily: 'var(--font-mono)',
                      }}
                      title={e.timestamp}
                    >
                      {fmtTime(e.timestamp)}
                    </span>
                    <span
                      style={{
                        color: 'var(--fg)',
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                      }}
                      title={e.summary || e.event_type}
                    >
                      {titleByRowId.get(e.id)}
                    </span>
                    <code
                      style={{
                        color: TONE_VAR[tone],
                        background: 'var(--surface)',
                        border: `1px solid var(--border)`,
                        borderRadius: 'var(--r-pill)',
                        padding: '0 6px',
                        fontSize: 10,
                        letterSpacing: '0.04em',
                        fontFamily: 'var(--font-mono)',
                        whiteSpace: 'nowrap',
                      }}
                      title={`event_type: ${e.event_type}`}
                    >
                      {e.event_type}
                    </code>
                    <span
                      style={{
                        fontSize: 10,
                        fontFamily: 'var(--font-mono)',
                        color: e.synced ? 'var(--fg-dim)' : 'var(--warn)',
                        letterSpacing: '0.06em',
                        textTransform: 'uppercase',
                      }}
                      title={e.synced ? 'Synced to remote' : 'Pending sync'}
                    >
                      {e.synced ? '✓' : '↑'}
                    </span>
                  </li>
                );
              })}
            </ol>
          </TrayCard>
        ))
      )}

      {/* LOAD MORE
          Server told us there are older rows behind the cursor. Tapping
          pages backwards and appends — the existing pages stay put so
          scroll position doesn't jump and the open drawer (if any)
          stays openable. Hidden when we've reached the tail. */}
      {hasMore && pages.length > 0 && (
        <div style={{ display: 'flex', justifyContent: 'center', padding: '4px 0' }}>
          <GhostButton
            type="button"
            onClick={() => {
              void handleLoadMore();
            }}
            disabled={loadingMore}
          >
            {loadingMore ? 'Loading more…' : 'Load more'}
          </GhostButton>
        </div>
      )}

      {/* DETAIL DRAWER
          Portaled to <body> so it escapes the App.tsx
          `.ss-screen-enter` ancestor, which has a CSS `transform`
          animation. A `transform` on an ancestor creates a containing
          block for `position: fixed` descendants, which would pin the
          drawer to the animating wrapper instead of the viewport. */}
      {selected && createPortal(
        <div onClick={() => setSelectedId(null)} style={SCRIM_STYLE}>
          <div
            onClick={(e) => e.stopPropagation()}
            role="dialog"
            aria-modal="true"
            style={DRAWER_STYLE}
          >
            <div
              style={{
                display: 'flex',
                alignItems: 'flex-start',
                justifyContent: 'space-between',
                gap: 12,
                marginBottom: 14,
              }}
            >
              <div>
                <div
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    gap: 8,
                    marginBottom: 4,
                  }}
                >
                  <StatusDot tone={toneForType(selected.event_type)} />
                  <code
                    style={{
                      color: TONE_VAR[toneForType(selected.event_type)],
                      fontFamily: 'var(--font-mono)',
                      fontSize: 14,
                      fontWeight: 600,
                      textTransform: 'uppercase',
                      letterSpacing: '0.04em',
                    }}
                  >
                    {selected.event_type}
                  </code>
                </div>
                <div style={{ fontSize: 14, color: 'var(--fg)' }}>
                  {selected.summary}
                </div>
              </div>
              <button
                type="button"
                onClick={() => setSelectedId(null)}
                style={{
                  background: 'transparent',
                  border: '1px solid var(--border-strong)',
                  color: 'var(--fg-muted)',
                  borderRadius: 'var(--r-sm)',
                  padding: '4px 10px',
                  fontSize: 12,
                  cursor: 'pointer',
                  fontFamily: 'inherit',
                }}
              >
                Close
              </button>
            </div>

            <dl
              style={{
                display: 'grid',
                gridTemplateColumns: '120px 1fr',
                gap: '6px 12px',
                margin: '0 0 14px',
              }}
            >
              <KV label="Event ID" value={`#${selected.id}`} mono />
              <KV
                label="Captured"
                value={`${fmtDate(selected.timestamp)} · ${fmtTime(
                  selected.timestamp,
                )} (${ageLabel(selected.timestamp)})`}
                mono
              />
              <KV
                label="Sync state"
                value={
                  <span
                    style={{
                      display: 'inline-flex',
                      alignItems: 'center',
                      gap: 6,
                      fontFamily: 'var(--font-mono)',
                      fontSize: 12,
                      color: selected.synced ? 'var(--ok)' : 'var(--warn)',
                    }}
                  >
                    <StatusDot tone={selected.synced ? 'ok' : 'warn'} />
                    {selected.synced
                      ? 'Synced to remote'
                      : 'Pending — will retry next batch'}
                  </span>
                }
              />
              <KV
                label="Source"
                value={`${selected.log_source.toUpperCase()}/Game.log`}
                mono
                dim
              />
            </dl>

            <div
              style={{
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'space-between',
                marginBottom: 6,
              }}
            >
              <div
                style={{
                  fontSize: 10,
                  fontWeight: 600,
                  color: 'var(--fg-muted)',
                  textTransform: 'uppercase',
                  letterSpacing: '0.12em',
                }}
              >
                Raw line
              </div>
              <button
                type="button"
                onClick={() => setShowRaw((v) => !v)}
                style={{
                  background: 'transparent',
                  border: '1px solid var(--border-strong)',
                  color: 'var(--fg-muted)',
                  borderRadius: 'var(--r-xs)',
                  padding: '2px 8px',
                  fontSize: 10,
                  cursor: 'pointer',
                  fontFamily: 'inherit',
                }}
              >
                {showRaw ? 'Hide' : 'Show'}
              </button>
            </div>
            {showRaw && (
              <pre
                style={{
                  margin: 0,
                  padding: '10px 12px',
                  background: 'var(--bg)',
                  border: '1px solid var(--border)',
                  borderRadius: 'var(--r-sm)',
                  color: 'var(--fg-muted)',
                  fontSize: 11,
                  fontFamily: 'var(--font-mono)',
                  whiteSpace: 'pre-wrap',
                  wordBreak: 'break-all',
                }}
              >
                {selected.raw_line}
              </pre>
            )}

            {noiseError && (
              <p
                role="alert"
                style={{
                  margin: '10px 0 0',
                  fontSize: 12,
                  color: 'var(--danger)',
                }}
              >
                Couldn&apos;t mark as noise: {noiseError}
              </p>
            )}

            <div
              style={{
                display: 'flex',
                gap: 8,
                marginTop: 14,
                paddingTop: 14,
                borderTop: '1px solid var(--border)',
              }}
            >
              <GhostButton type="button" onClick={handleCopyRaw}>
                {copyState === 'copied'
                  ? 'Copied'
                  : copyState === 'failed'
                    ? 'Copy failed'
                    : 'Copy raw line'}
              </GhostButton>
              <GhostButton
                type="button"
                onClick={handleMarkAsNoise}
                disabled={pendingNoise}
              >
                {pendingNoise ? 'Marking…' : 'Mark as noise'}
              </GhostButton>
              {!selected.synced && (
                <GhostButton
                  type="button"
                  onClick={handleRetrySync}
                  disabled={retryPending}
                  title="Wake the sync worker now instead of waiting for the next tick"
                >
                  {retryState === 'kicked'
                    ? 'Sync nudged'
                    : retryState === 'failed'
                      ? 'Retry failed'
                      : retryPending
                        ? 'Nudging…'
                        : 'Retry sync'}
                </GhostButton>
              )}
            </div>
          </div>
        </div>,
        document.body,
      )}

      <style>{`
        @keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } }
        @keyframes slideUp {
          from { transform: translateY(20px); opacity: 0; }
          to { transform: translateY(0); opacity: 1; }
        }
      `}</style>
    </div>
  );
}
