import { useEffect, useMemo, useState } from 'react';
import {
  api,
  type LogKind,
  type ParseCoverageResponse,
  type ReleaseChannel,
  type SettingsField,
  type SourceStats,
  type StatusResponse,
  type TimelineEntry,
} from '../api';
import {
  Banner,
  GhostButton,
  KV,
  StatPill,
  StatusDot,
  TrayCard,
} from './tray/primitives';
import { UpdateBanner } from './UpdateBanner';
import {
  ageLabel,
  fmtBytes,
  fmtCovPct,
  fmtTime,
  humanizeEventType,
  humanTitleForEntry,
  toneForType,
  TONE_VAR,
} from './tray/format';
import { humanTitleForEntryReact } from './tray/format-react';
import type { HangarStats } from '../api';
import { EventSparkline } from './EventSparkline';
import { HealthCard } from './HealthCard';
import { useHealth } from '../hooks/useHealth';
import { friendlyError } from '../lib/friendlyError';
import { isInTransitNoise } from '../lib/inTransitFilter';
import { composeProfileUrl } from '../lib/profileUrl';
import { TrayEntityLink } from './kb/TrayEntityLink';

/// Compact label + tone colour for each `LogKind`. Live = ok green
/// (it's the currently-tailed source); archived = info blue (passive
/// historical); crash = warn (forensic); launcher = neutral (not
/// gameplay).
const KIND_META: Record<LogKind, { label: string; tone: string }> = {
  channel_live: { label: 'LIVE', tone: 'var(--ok)' },
  channel_archived: { label: 'ARCH', tone: 'var(--info)' },
  crash_report: { label: 'CRASH', tone: 'var(--warn)' },
  launcher_log: { label: 'LNCH', tone: 'var(--fg-muted)' },
};

/// Display order for the "Discovered logs" kind breakdown chips.
/// Mirrors the declaration order of KIND_META so a freshly-added kind
/// also picks up a chip slot without code changes elsewhere — but
/// keeping the explicit array means the order is intentional, not
/// accidentally object-key-iteration-dependent.
const KIND_ORDER: LogKind[] = [
  'channel_live',
  'channel_archived',
  'crash_report',
  'launcher_log',
];

/// Group discovered logs by `kind` and return one row per non-empty
/// bucket. Used by the Discovered logs card to render a compact
/// breakdown chip row instead of a per-file list.
function kindBreakdown(
  logs: ReadonlyArray<{ kind: LogKind }>
): Array<{ kind: LogKind; count: number }> {
  const counts = new Map<LogKind, number>();
  for (const log of logs) {
    counts.set(log.kind, (counts.get(log.kind) ?? 0) + 1);
  }
  return KIND_ORDER.flatMap((kind) => {
    const count = counts.get(kind);
    return count ? [{ kind, count }] : [];
  });
}

interface Props {
  status: StatusResponse;
  /// Current release channel — drives the top-of-pane update check.
  channel: ReleaseChannel;
  /// The user's "auto-check for updates" preference; gates the proactive
  /// on-mount check (the manual button always works regardless).
  autoCheck: boolean;
  /// Web UI origin used to build the "Open on web" deep link to the
  /// user's public profile (`${webOrigin}/u/${claimed_handle}`).
  /// `null` until the device is paired and the API URL is configured.
  webOrigin: string | null;
  /// Routes the user to the Settings pane and focuses a specific
  /// field. Driven by HealthCard CTAs (e.g. "Set up" → API URL).
  onGoToSettings: (field: SettingsField) => void;
  /// Catalog-driven class-name prettifier. Wires the synced
  /// reference catalogue into the recent-activity timeline so raw
  /// engine identifiers (AEGS_Avenger_Stalker) render as friendly
  /// display names. Optional — falls through to the raw
  /// Rust-formatted summary if absent.
  prettyLookup?: import('./tray/format').PrettyLookup;
  /// Full catalogue bundles. When supplied alongside `webOrigin`,
  /// event-summary rendering swaps the plain-text prettifier for a
  /// ReactNode variant that wraps each class-name token in
  /// `<TrayEntityLink>` so the user can click through to the KB
  /// page in the web app. Optional — falls through to the
  /// `prettyLookup` string path when missing.
  bundles?: import('../lib/reference').AllReferenceBundles;
}

/// Transient label flashed on the "Copy summary" button after a
/// clipboard write. Reverts to `idle` automatically.
type CopyState = 'idle' | 'copied' | 'failed';

const COPY_FLASH_OK_MS = 1500;
const COPY_FLASH_ERR_MS = 2000;

export function StatusPane({
  status,
  webOrigin,
  onGoToSettings,
  prettyLookup,
  bundles,
  channel,
  autoCheck,
}: Props) {
  const {
    tail,
    sync,
    event_counts,
    total_events,
    discovered_logs,
    hangar,
  } = status;
  const { items: healthItems, refresh: refreshHealth } = useHealth();
  const [coverage, setCoverage] = useState<ParseCoverageResponse | null>(
    null,
  );
  const [timeline, setTimeline] = useState<TimelineEntry[] | null>(null);
  const [sourceStats, setSourceStats] = useState<SourceStats | null>(null);
  // Per-row mark-as-noise error message keyed by event_name. A user
  // who clicks "ignore" expects the row to disappear; if the mutation
  // fails we have to say so, otherwise they'll click again and again
  // while the row stays put.
  const [noiseErrors, setNoiseErrors] = useState<Record<string, string>>({});
  // Set of event_names with an in-flight mark-as-noise call, to
  // disable double-clicks and surface a "working…" affordance.
  const [pendingNoise, setPendingNoise] = useState<Set<string>>(new Set());
  // Copy-summary button: prevents reentrancy and drives the transient
  // "Copied!" / "Failed" label that auto-reverts after a beat.
  const [copyState, setCopyState] = useState<CopyState>('idle');
  const [copyInFlight, setCopyInFlight] = useState(false);
  // Claimed RSI handle, fetched lazily on mount via `getConfig`.
  // Stays null on failure so the Open-on-web button renders disabled
  // rather than throwing — the user can still pair from Settings.
  const [claimedHandle, setClaimedHandle] = useState<string | null>(null);
  // Free-text filter over the session timeline; matches against
  // event_type and summary, case-insensitive.
  const [timelineQuery, setTimelineQuery] = useState('');

  useEffect(() => {
    let cancelled = false;

    const fetchAll = async () => {
      try {
        const [cov, tl, ss] = await Promise.all([
          api.getParseCoverage(),
          api.getSessionTimeline(),
          api.getSourceStats(),
        ]);
        if (!cancelled) {
          setCoverage(cov);
          setTimeline(tl);
          setSourceStats(ss);
        }
      } catch (err) {
        // Keep UI quiet on failure; surface to dev console only.
        // eslint-disable-next-line no-console
        console.warn('Failed to refresh status data', err);
      }
    };

    fetchAll();
    const handle = window.setInterval(fetchAll, 15_000);

    return () => {
      cancelled = true;
      window.clearInterval(handle);
    };
  }, []);

  // One-shot config read on mount to populate the claimed handle used
  // by the "Open on web" CTA. Swallows failures: a null handle just
  // renders the button disabled.
  useEffect(() => {
    api
      .getConfig()
      .then((c) => setClaimedHandle(c.remote_sync.claimed_handle))
      .catch(() => {});
  }, []);

  const profileUrl = composeProfileUrl(webOrigin, claimedHandle);

  const handleCopySummary = async () => {
    if (copyInFlight) return;
    setCopyInFlight(true);
    try {
      const text = await api.getSessionSummaryText();
      await navigator.clipboard.writeText(text);
      setCopyState('copied');
      window.setTimeout(() => setCopyState('idle'), COPY_FLASH_OK_MS);
    } catch (e) {
      setCopyState('failed');
      const f = friendlyError(e);
      // eslint-disable-next-line no-console
      console.warn('copy_session_summary failed:', f.title, f.body);
      window.setTimeout(() => setCopyState('idle'), COPY_FLASH_ERR_MS);
    } finally {
      setCopyInFlight(false);
    }
  };

  const handleOpenOnWeb = async () => {
    if (!profileUrl) return;
    try {
      const { open } = await import('@tauri-apps/plugin-shell');
      await open(profileUrl);
    } catch (e) {
      const f = friendlyError(e);
      // eslint-disable-next-line no-console
      console.warn('open_on_web failed:', f.title, f.body);
    }
  };

  const handleMarkAsNoise = async (eventName: string) => {
    setPendingNoise((prev) => {
      const next = new Set(prev);
      next.add(eventName);
      return next;
    });
    setNoiseErrors((prev) => {
      if (!(eventName in prev)) return prev;
      const { [eventName]: _removed, ...rest } = prev;
      return rest;
    });
    try {
      await api.markEventAsNoise(eventName);
      const refreshed = await api.getParseCoverage();
      setCoverage(refreshed);
    } catch (err) {
      // Tauri invoke rejections are plain strings; friendlyError handles
      // strings and yields readable copy (M-U3).
      const message = friendlyError(err).body;
      setNoiseErrors((prev) => ({ ...prev, [eventName]: message }));
    } finally {
      setPendingNoise((prev) => {
        const next = new Set(prev);
        next.delete(eventName);
        return next;
      });
    }
  };

  const hasSyncActivity =
    sync.last_attempt_at !== null || sync.batches_sent > 0;

  // Top-types ranked bar — denominator clamped to 1 to avoid
  // divide-by-zero when event_counts is empty (handled separately
  // below, but still defensive).
  const maxCount = Math.max(...event_counts.map((c) => c.count), 1);

  const totalSyncEvents =
    sync.events_accepted + sync.events_duplicate + sync.events_rejected;

  // In-Transit movement noise is hidden from the friendly activity timeline
  // to match web (the "hidden in BOTH apps" invariant). Display-only — the
  // events still persist and remain visible in the raw Logs view. This is the
  // display universe for both the filter and the entry counts (M-U1).
  const shownTimeline = useMemo(
    () =>
      timeline
        ? timeline.filter((e) => !isInTransitNoise(e.event_type))
        : null,
    [timeline],
  );

  // In-memory timeline filter — matches event_type and summary,
  // case-insensitive. Returns the full (suppressed) list when blank.
  const visibleTimeline = useMemo(() => {
    if (!shownTimeline) return null;
    const q = timelineQuery.trim().toLowerCase();
    if (!q) return shownTimeline;
    return shownTimeline.filter(
      (e) =>
        e.event_type.toLowerCase().includes(q) ||
        e.summary.toLowerCase().includes(q),
    );
  }, [shownTimeline, timelineQuery]);

  const isFiltering = timelineQuery.trim().length > 0;
  const timelineKicker =
    shownTimeline === null
      ? '…'
      : isFiltering && visibleTimeline
        ? `${visibleTimeline.length} of ${shownTimeline.length} entries`
        : `${shownTimeline.length} entries`;

  // Player-value headline derivations. All three come from data
  // already in scope so the new pills cost no extra fetch:
  //   - events48h: how active the player has been recently
  //   - topType:   what they're doing most of
  //   - ships:     hangar fleet size (the closest single number to
  //                "your account is worth this much in-game")
  const events48h = useMemo(() => {
    if (!timeline) return null;
    const cutoff = Date.now() - 48 * 60 * 60 * 1000;
    return timeline.filter(
      (e) => new Date(e.timestamp).getTime() >= cutoff,
    ).length;
  }, [timeline]);
  // H9: never surface the raw snake_case event_type as a headline; run it
  // through humanizeEventType (verb table → sentence-cased fallback).
  const topTypeRaw = event_counts[0]?.event_type ?? null;
  const topType = topTypeRaw ? humanizeEventType(topTypeRaw) : '—';

  const copyLabel =
    copyState === 'copied'
      ? 'Copied!'
      : copyState === 'failed'
        ? 'Failed — see log'
        : 'Copy summary';

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      {/* Unpaired devices get one prominent, actionable CTA at the very
          top instead of relying on scattered inline hints. `webOrigin`
          is null until the device is paired and its API URL is set, so
          it's the right gate. The action jumps straight to the pairing
          field in Settings. */}
      {webOrigin === null && (
        <Banner
          tone="info"
          action="Pair this device"
          onAction={() => onGoToSettings('pairing_code')}
        >
          This device isn’t paired yet. Pair it to start syncing your events
          and unlock the web dashboard.
        </Banner>
      )}
      <UpdateBanner channel={channel} autoCheck={autoCheck} />
      <HealthCard
        items={healthItems}
        onGoToSettings={onGoToSettings}
        onDismiss={async (id) => {
          try {
            await api.dismissHealth(id);
            void refreshHealth();
          } catch (e) {
            const f = friendlyError(e);
            // eslint-disable-next-line no-console
            console.warn('dismiss_health failed:', f.title, f.body);
          }
        }}
        onRetrySync={async () => {
          try {
            await api.retrySyncNow();
            void refreshHealth();
          } catch (e) {
            const f = friendlyError(e);
            // eslint-disable-next-line no-console
            console.warn('retry_sync_now failed:', f.title, f.body);
          }
        }}
        onRefreshHangar={async () => {
          try {
            await api.refreshHangarNow();
            void refreshHealth();
          } catch (e) {
            const f = friendlyError(e);
            // eslint-disable-next-line no-console
            console.warn('refresh_hangar_now failed:', f.title, f.body);
          }
        }}
        onOpenUrl={async (url) => {
          const { open } = await import('@tauri-apps/plugin-shell');
          await open(url);
        }}
      />

      {/* ACTION ROW */}
      <div style={{ display: 'flex', gap: 8 }}>
        <GhostButton
          onClick={handleCopySummary}
          disabled={copyInFlight}
          title="Copy a plain-text session summary to the clipboard"
        >
          {copyLabel}
        </GhostButton>
        <GhostButton
          onClick={handleOpenOnWeb}
          disabled={profileUrl === null}
          title={
            profileUrl === null
              ? 'Pair the device and configure web origin first'
              : 'Open your StarStats profile in the browser'
          }
        >
          Open on web
        </GhostButton>
      </div>

      {/* HEADLINE STAT STRIP — reframed around what the player cares
          about (recent activity, dominant event, fleet size) instead
          of pipeline counters. Pipeline numbers (lines/batches/
          coverage) live in the collapsed Pipeline section below. */}
      <div style={{ display: 'flex', gap: 8 }}>
        <StatPill
          label="Events"
          value={total_events.toLocaleString()}
          tone="accent"
        />
        <StatPill
          label="48h"
          value={events48h === null ? '—' : events48h.toLocaleString()}
        />
        <StatPill
          label="Top type"
          value={topType}
        />
        <StatPill
          label="Ships"
          value={hangar.ships_pushed.toLocaleString()}
          tone="ok"
        />
      </div>

      {/* 48-hour activity sparkline — buckets the in-memory timeline
          into hourly counts. Same data as Top types below, just shaped
          for "when did stuff happen" instead of "what kinds happened". */}
      {timeline && timeline.length > 0 ? (
        <TrayCard title="Recent activity" kicker="48h">
          <EventSparkline entries={timeline} />
        </TrayCard>
      ) : null}

      {/* TOP TYPES */}
      {event_counts.length === 0 ? (
        <TrayCard title="Top event types">
          <p style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13 }}>
            No events captured yet.
          </p>
        </TrayCard>
      ) : (
        <TrayCard
          title="Top event types"
          kicker={`${event_counts.length} types`}
        >
          <div
            style={{ display: 'flex', flexDirection: 'column', gap: 5 }}
          >
            {event_counts.map((c) => {
              const pct = (c.count / maxCount) * 100;
              return (
                <div
                  key={c.event_type}
                  style={{
                    display: 'grid',
                    gridTemplateColumns: '1fr auto',
                    gap: 8,
                    alignItems: 'center',
                    position: 'relative',
                  }}
                >
                  <div style={{ position: 'relative' }}>
                    <div
                      style={{
                        position: 'absolute',
                        inset: 0,
                        background: 'var(--surface-2)',
                        borderRadius: 'var(--r-xs)',
                      }}
                    />
                    <div
                      style={{
                        position: 'absolute',
                        top: 0,
                        bottom: 0,
                        left: 0,
                        width: `${pct}%`,
                        background:
                          'linear-gradient(90deg, var(--accent-soft) 0%, transparent 100%)',
                        borderRadius: 'var(--r-xs)',
                        borderLeft: '2px solid var(--accent)',
                      }}
                    />
                    <code
                      style={{
                        position: 'relative',
                        display: 'block',
                        padding: '4px 8px',
                        fontSize: 11.5,
                        color: 'var(--fg)',
                        fontFamily: 'var(--font-mono)',
                      }}
                    >
                      {c.event_type}
                    </code>
                  </div>
                  <span
                    style={{
                      fontFamily: 'var(--font-mono)',
                      fontSize: 12,
                      color: 'var(--fg-muted)',
                      fontVariantNumeric: 'tabular-nums',
                      minWidth: 44,
                      textAlign: 'right',
                    }}
                  >
                    {c.count.toLocaleString()}
                  </span>
                </div>
              );
            })}
          </div>
        </TrayCard>
      )}

      {/* TIMELINE */}
      <TrayCard
        title="Session timeline"
        kicker={timelineKicker}
        right={
          <input
            type="search"
            value={timelineQuery}
            onChange={(ev) => setTimelineQuery(ev.target.value)}
            placeholder="filter…"
            aria-label="Filter session timeline"
            style={{
              width: 140,
              padding: '4px 8px',
              fontSize: 11,
              fontFamily: 'inherit',
              border: '1px solid var(--border)',
              background: 'var(--bg)',
              color: 'var(--fg)',
              borderRadius: 'var(--r-xs)',
            }}
          />
        }
      >
        {visibleTimeline === null ? (
          <p style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13 }}>
            Loading timeline…
          </p>
        ) : visibleTimeline.length === 0 ? (
          <p style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13 }}>
            {isFiltering
              ? `No entries match “${timelineQuery.trim()}”.`
              : 'Scope is clear. Launch Star Citizen to start the feed.'}
          </p>
        ) : (
          <ol
            style={{
              listStyle: 'none',
              margin: 0,
              padding: 0,
              display: 'flex',
              flexDirection: 'column',
              gap: 2,
              maxHeight: 300,
              overflowY: 'auto',
            }}
          >
            {visibleTimeline.map((e) => {
              const accent = TONE_VAR[toneForType(e.event_type)];
              return (
                <li
                  key={e.id}
                  style={{
                    display: 'grid',
                    gridTemplateColumns: '60px 1fr auto',
                    gap: 10,
                    alignItems: 'baseline',
                    padding: '4px 6px',
                    borderLeft: `2px solid ${accent}`,
                    fontSize: 12,
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
                      display: 'flex',
                      alignItems: 'baseline',
                      gap: 6,
                      minWidth: 0,
                    }}
                    title={e.summary || e.event_type}
                  >
                    <span
                      style={{
                        overflow: 'hidden',
                        textOverflow: 'ellipsis',
                        whiteSpace: 'nowrap',
                        minWidth: 0,
                      }}
                    >
                      {bundles && webOrigin
                        ? humanTitleForEntryReact(e, bundles, webOrigin)
                        : humanTitleForEntry(e, prettyLookup)}
                    </span>
                    {/* Client-side resolved location → KB link (or
                        plain friendly name when not confident). */}
                    {e.location ? (
                      <span
                        style={{
                          flexShrink: 0,
                          color: 'var(--fg-dim)',
                          fontSize: 11,
                        }}
                      >
                        <span aria-hidden>📍</span>{' '}
                        <TrayEntityLink
                          category="location"
                          classKey={e.location.slug ?? e.location.display_name}
                          resolved={{
                            slug: e.location.slug ?? null,
                            displayName: e.location.display_name,
                          }}
                          webOrigin={webOrigin}
                        />
                      </span>
                    ) : null}
                  </span>
                  <code
                    style={{
                      color: accent,
                      background: 'var(--surface)',
                      border: '1px solid var(--border)',
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
                </li>
              );
            })}
          </ol>
        )}
      </TrayCard>

      {/* HANGAR (promoted — second-most-important card after Health
          for a player view; "what's in my fleet?" deserves to live
          above the pipeline diagnostics). */}
      <HangarCard hangar={hangar} />

      {/* PIPELINE — collapsed by default. Mechanical pipeline cards
          (feed health, parser quality, source breakdowns); the
          HealthCard above already surfaces anything broken, so these
          are for curiosity and debugging rather than day-to-day use. */}
      <PipelineSection
        tail={tail}
        sync={sync}
        hasSyncActivity={hasSyncActivity}
        totalSyncEvents={totalSyncEvents}
        coverage={coverage}
        discoveredLogs={discovered_logs}
        sourceStats={sourceStats}
        pendingNoise={pendingNoise}
        noiseErrors={noiseErrors}
        onMarkAsNoise={handleMarkAsNoise}
      />
    </div>
  );
}

interface PipelineSectionProps {
  tail: StatusResponse['tail'];
  sync: StatusResponse['sync'];
  hasSyncActivity: boolean;
  totalSyncEvents: number;
  coverage: ParseCoverageResponse | null;
  discoveredLogs: StatusResponse['discovered_logs'];
  sourceStats: SourceStats | null;
  pendingNoise: Set<string>;
  noiseErrors: Record<string, string>;
  onMarkAsNoise: (eventName: string) => void;
}

/// Collapsible "Pipeline" diagnostics section — bundles the four
/// mechanical-pipeline cards (Tailing + Remote sync grid, Parser
/// coverage, Discovered logs, Sources) behind a single `<details>`
/// disclosure so the day-to-day player view stays focused on what
/// they did in-game rather than how the log scraper is feeling.
function PipelineSection({
  tail,
  sync,
  hasSyncActivity,
  totalSyncEvents,
  coverage,
  discoveredLogs,
  sourceStats,
  pendingNoise,
  noiseErrors,
  onMarkAsNoise,
}: PipelineSectionProps) {
  return (
    <details>
      {/* Single, one-per-pane section category label sitting directly atop
          this collapsible section's contents — the sanctioned `.ss-eyebrow`
          "section category label" use (L8 ruling). Reclassed onto the
          shared class (Michroma) instead of a hand-rolled uppercase inline
          style so it matches web (M4). */}
      <summary
        className="ss-eyebrow"
        style={{
          cursor: 'pointer',
          padding: '4px 0',
          userSelect: 'none',
        }}
      >
        Pipeline · feed and parser internals
      </summary>
      <div
        style={{
          display: 'flex',
          flexDirection: 'column',
          gap: 12,
          marginTop: 8,
        }}
      >
        {/* TAILING + SYNC, side-by-side */}
        <div
          style={{
            display: 'grid',
            gridTemplateColumns: '1fr 1fr',
            gap: 12,
          }}
        >
          <TrayCard
            title="Tailing"
            kicker={tail.current_path ? 'LIVE' : 'IDLE'}
          >
            {tail.current_path ? (
              <dl
                style={{
                  display: 'grid',
                  gridTemplateColumns: '90px 1fr',
                  gap: '6px 10px',
                  margin: 0,
                }}
              >
                <KV label="Path" value={tail.current_path} mono />
                <KV label="Read" value={fmtBytes(tail.bytes_read)} />
                <KV
                  label="Last event"
                  value={
                    tail.last_event_type ? (
                      <>
                        <code
                          style={{ color: 'var(--accent)', fontSize: 12 }}
                        >
                          {tail.last_event_type}
                        </code>{' '}
                        <span style={{ color: 'var(--fg-dim)' }}>
                          · {fmtTime(tail.last_event_at)}
                        </span>
                      </>
                    ) : (
                      <span style={{ color: 'var(--fg-dim)' }}>—</span>
                    )
                  }
                />
              </dl>
            ) : (
              <p
                style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13 }}
              >
                Scope is clear. No Game.log discovered yet — set a custom
                path in Settings.
              </p>
            )}
          </TrayCard>

          <TrayCard
            title="Remote sync"
            right={
              <span
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 6,
                  fontSize: 11,
                  color: sync.last_error ? 'var(--danger)' : 'var(--ok)',
                }}
              >
                <StatusDot tone={sync.last_error ? 'danger' : 'ok'} />
                <span
                  style={{
                    fontFamily: 'var(--font-mono)',
                    textTransform: 'uppercase',
                    letterSpacing: '0.08em',
                  }}
                >
                  {sync.last_error ? 'ERR' : 'OK'}
                </span>
              </span>
            }
          >
            {hasSyncActivity ? (
              <dl
                style={{
                  display: 'grid',
                  gridTemplateColumns: '90px 1fr',
                  gap: '6px 10px',
                  margin: 0,
                }}
              >
                <KV
                  label="Last sync"
                  value={
                    sync.last_success_at ? fmtTime(sync.last_success_at) : '—'
                  }
                  mono
                />
                <KV
                  label="Accepted"
                  value={`${sync.events_accepted.toLocaleString()} / ${totalSyncEvents.toLocaleString()}`}
                  mono
                />
                <KV
                  label="Dup / rej"
                  value={`${sync.events_duplicate} · ${sync.events_rejected}`}
                  mono
                  dim
                />
              </dl>
            ) : (
              <p
                style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13 }}
              >
                Disabled. Configure remote sync in Settings to push events
                to an API server.
              </p>
            )}
          </TrayCard>
        </div>

        {/* PARSER COVERAGE */}
        <TrayCard title="Parser coverage">
        {coverage === null ? (
          <p style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13 }}>
            Loading parser coverage…
          </p>
        ) : (
          <>
            <div
              style={{
                display: 'flex',
                alignItems: 'baseline',
                gap: 12,
                marginBottom: 6,
              }}
            >
              <span
                style={{
                  fontFamily: 'var(--font-mono)',
                  fontSize: 28,
                  fontWeight: 700,
                  color: 'var(--accent)',
                  fontVariantNumeric: 'tabular-nums',
                  lineHeight: 1,
                }}
              >
                {fmtCovPct(coverage.recognised, coverage.structural_only)}
              </span>
              <span style={{ fontSize: 12, color: 'var(--fg-muted)' }}>
                recognised lines
              </span>
            </div>
            <div
              style={{
                fontSize: 11,
                color: 'var(--fg-dim)',
                fontFamily: 'var(--font-mono)',
              }}
            >
              {coverage.recognised.toLocaleString()} ok ·{' '}
              {coverage.structural_only.toLocaleString()} unknown ·{' '}
              {coverage.noise.toLocaleString()} noise ·{' '}
              {coverage.skipped.toLocaleString()} skipped
            </div>

            {coverage.unknowns.length === 0 ? (
              <p
                style={{
                  margin: '10px 0 0',
                  color: 'var(--fg-dim)',
                  fontSize: 13,
                }}
              >
                Nothing unrecognized yet — your parser rules cover
                everything we&apos;ve seen.
              </p>
            ) : (
              <details style={{ marginTop: 10 }}>
                <summary
                  style={{
                    cursor: 'pointer',
                    color: 'var(--fg-muted)',
                    fontSize: 12,
                    padding: '4px 0',
                  }}
                >
                  Unrecognised types ({coverage.unknowns.length})
                </summary>
                <ul
                  style={{
                    listStyle: 'none',
                    padding: 0,
                    margin: '8px 0 0',
                    display: 'flex',
                    flexDirection: 'column',
                    gap: 6,
                  }}
                >
                  {coverage.unknowns.map((u) => (
                    <li
                      key={`${u.log_source}:${u.event_name}`}
                      style={{
                        border: '1px solid var(--border)',
                        borderRadius: 'var(--r-sm)',
                        padding: '6px 8px',
                        background: 'var(--bg)',
                      }}
                    >
                      <div
                        style={{
                          display: 'grid',
                          gridTemplateColumns: 'auto 1fr auto auto',
                          gap: 8,
                          alignItems: 'center',
                          fontSize: 11.5,
                        }}
                      >
                        <span
                          style={{
                            background: 'var(--accent-soft)',
                            color: 'var(--accent)',
                            fontSize: 10,
                            fontWeight: 700,
                            padding: '1px 6px',
                            borderRadius: 'var(--r-xs)',
                            letterSpacing: '0.04em',
                          }}
                        >
                          {u.log_source}
                        </span>
                        <code
                          style={{
                            color: 'var(--accent)',
                            fontFamily: 'var(--font-mono)',
                            wordBreak: 'break-all',
                          }}
                        >
                          {u.event_name}
                        </code>
                        <span
                          style={{
                            color: 'var(--fg-muted)',
                            fontFamily: 'var(--font-mono)',
                            fontVariantNumeric: 'tabular-nums',
                          }}
                        >
                          {u.occurrences.toLocaleString()}
                        </span>
                        <button
                          type="button"
                          disabled={pendingNoise.has(u.event_name)}
                          onClick={() => onMarkAsNoise(u.event_name)}
                          title="Add to noise list — drops the existing sample and stops recording new ones"
                          style={{
                            background: 'transparent',
                            border: '1px solid var(--border-strong)',
                            color: 'var(--fg-muted)',
                            borderRadius: 'var(--r-xs)',
                            padding: '1px 6px',
                            fontSize: 10,
                            cursor: pendingNoise.has(u.event_name)
                              ? 'not-allowed'
                              : 'pointer',
                            opacity: pendingNoise.has(u.event_name)
                              ? 0.55
                              : 1,
                            fontFamily: 'inherit',
                          }}
                        >
                          {pendingNoise.has(u.event_name)
                            ? 'working…'
                            : 'ignore'}
                        </button>
                      </div>
                      {noiseErrors[u.event_name] && (
                        <p
                          role="alert"
                          style={{
                            margin: '4px 0 0',
                            fontSize: 11,
                            color: 'var(--danger)',
                          }}
                        >
                          Couldn&apos;t mark as noise:{' '}
                          {noiseErrors[u.event_name]}
                        </p>
                      )}
                    </li>
                  ))}
                </ul>
              </details>
            )}
          </>
        )}
      </TrayCard>

      {/* DISCOVERED LOGS */}
      <TrayCard
        title="Discovered logs"
        kicker={`${discoveredLogs.length} found`}
      >
        {discoveredLogs.length === 0 ? (
          <p style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13 }}>
            No Game.log files discovered.
          </p>
        ) : (
          <>
            <p style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13 }}>
              All {discoveredLogs.length} discovered{' '}
              {discoveredLogs.length === 1 ? 'log is' : 'logs are'} being read;
              events from each are included in the tray pipeline.
            </p>
            <div
              style={{
                display: 'flex',
                flexWrap: 'wrap',
                gap: 6,
                marginTop: 8,
              }}
            >
              {kindBreakdown(discoveredLogs).map(({ kind, count }) => {
                const meta = KIND_META[kind];
                return (
                  <span
                    key={kind}
                    title={kind}
                    style={{
                      background: meta.tone,
                      color: 'var(--bg)',
                      fontSize: 10,
                      fontWeight: 700,
                      padding: '2px 6px',
                      borderRadius: 'var(--r-xs)',
                      letterSpacing: '0.06em',
                    }}
                  >
                    {count} {meta.label}
                  </span>
                );
              })}
            </div>
          </>
        )}
      </TrayCard>

      {/* SOURCES */}
      <SourcesCard stats={sourceStats} />

      </div>
    </details>
  );
}

/// "Sources" card — surfaces the secondary-pipeline stats (launcher
/// tail, crash scanner, rotated-log backfill) that the tray now runs
/// alongside the live Game.log tail. None of these are blocking; the
/// card just confirms each pipeline is doing something so the user
/// can spot a stuck one.
function SourcesCard({ stats }: { stats: SourceStats | null }) {
  if (!stats) {
    return (
      <TrayCard title="Sources" kicker="loading…">
        <p style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13 }}>
          Reading pipeline stats…
        </p>
      </TrayCard>
    );
  }

  const { launcher, crashes, backfill } = stats;
  const backfillKicker = backfill.completed
    ? `${backfill.files_processed + backfill.files_already_done}/${backfill.files_total} files`
    : `scanning… ${backfill.files_processed}/${backfill.files_total}`;

  return (
    <TrayCard title="Sources" kicker={backfillKicker}>
      <dl
        style={{
          display: 'grid',
          gridTemplateColumns: '110px 1fr',
          gap: '6px 10px',
          margin: 0,
        }}
      >
        <KV
          label="Launcher"
          value={
            launcher.current_path === null ? (
              <span style={{ color: 'var(--fg-dim)' }}>not running</span>
            ) : (
              <span>
                {launcher.events_recognised.toLocaleString()} events
                {launcher.last_category && (
                  <>
                    {' · last: '}
                    <span
                      style={{
                        textTransform: 'uppercase',
                        fontSize: 10,
                        fontWeight: 700,
                        letterSpacing: '0.06em',
                        color:
                          launcher.last_category === 'error'
                            ? 'var(--danger)'
                            : 'var(--accent)',
                      }}
                    >
                      {launcher.last_category}
                    </span>
                  </>
                )}
              </span>
            )
          }
        />
        <KV
          label="Crashes"
          value={
            crashes.total_crashes_seen === 0 ? (
              <span style={{ color: 'var(--ok)' }}>none on disk</span>
            ) : (
              <span>
                {crashes.total_crashes_seen} on disk
                {crashes.last_crash_dir && (
                  <span
                    className="mono"
                    style={{
                      color: 'var(--fg-dim)',
                      fontSize: 11,
                      marginLeft: 6,
                    }}
                  >
                    last: {crashes.last_crash_dir}
                  </span>
                )}
              </span>
            )
          }
        />
        <KV
          label="Backfill"
          value={
            backfill.completed ? (
              <span>
                {backfill.events_recognised.toLocaleString()} events from{' '}
                {backfill.files_processed} archived
                {backfill.files_already_done > 0 && (
                  <span style={{ color: 'var(--fg-dim)' }}>
                    {' '}
                    ({backfill.files_already_done} already done)
                  </span>
                )}
              </span>
            ) : (
              <span style={{ color: 'var(--info)' }}>
                replaying historical sessions…
              </span>
            )
          }
        />
      </dl>
    </TrayCard>
  );
}

/// Render-state derived from `HangarStats`. Field combinations encode
/// six distinct UX states; the previous "ERROR else OK" kicker treated
/// "never ran" identically to "succeeded", which is what produced the
/// misleading "OK · no last sync" rendering. We always look at all
/// three timestamps (attempt/success/error) plus the skip reason.
type HangarState =
  | { kind: 'never_started' }
  | { kind: 'refreshing' }
  | { kind: 'fresh_success'; at: string; ageMs: number }
  | { kind: 'stale_success'; at: string }
  | { kind: 'skipped'; at: string | null; reason: string }
  | { kind: 'error'; at: string | null; message: string };

/// Anything fresher than this counts as a "just-fetched" affirmative
/// success (green dot). Past the window we still call it a success but
/// fade it to the muted "info" tone.
const HANGAR_FRESH_MS = 5 * 60_000;

/// Hard ceiling on the in-flight spinner. If the polled
/// `last_attempt_at` hasn't advanced past the click stamp by then,
/// we drop back to the previous state rather than spin forever — the
/// kick worker may not even be spawned (no api_url/token configured).
const HANGAR_REFRESH_TIMEOUT_MS = 60_000;

function deriveHangarState(
  h: HangarStats,
  refreshingSince: number | null,
): HangarState {
  if (refreshingSince !== null) {
    return { kind: 'refreshing' };
  }
  if (h.last_error) {
    return {
      kind: 'error',
      at: h.last_attempt_at,
      message: h.last_error,
    };
  }
  if (h.last_success_at) {
    const ageMs = Date.now() - new Date(h.last_success_at).getTime();
    if (ageMs < HANGAR_FRESH_MS) {
      return { kind: 'fresh_success', at: h.last_success_at, ageMs };
    }
    return { kind: 'stale_success', at: h.last_success_at };
  }
  if (h.last_skip_reason) {
    return {
      kind: 'skipped',
      at: h.last_attempt_at,
      reason: h.last_skip_reason,
    };
  }
  return { kind: 'never_started' };
}

interface HangarCardProps {
  hangar: HangarStats;
}

function HangarCard({ hangar }: HangarCardProps) {
  // Stamped on click; cleared once a polled `last_attempt_at` lands
  // past the stamp (see effect below). Without this, "Refresh now"
  // would leave no trace in the UI between clicks until the next
  // poll arrives.
  const [refreshingSince, setRefreshingSince] = useState<number | null>(null);
  const [refreshError, setRefreshError] = useState<string | null>(null);

  useEffect(() => {
    if (refreshingSince === null) return;

    const lastAttemptMs = hangar.last_attempt_at
      ? new Date(hangar.last_attempt_at).getTime()
      : 0;
    if (lastAttemptMs >= refreshingSince) {
      setRefreshingSince(null);
      return;
    }

    // Safety net: if no new attempt lands within the timeout, give
    // up so the spinner doesn't stick. The hangar worker only runs
    // when api_url + access_token are both set; the kick is a no-op
    // otherwise.
    const remaining =
      refreshingSince + HANGAR_REFRESH_TIMEOUT_MS - Date.now();
    const handle = window.setTimeout(
      () => setRefreshingSince(null),
      Math.max(0, remaining),
    );
    return () => window.clearTimeout(handle);
  }, [hangar.last_attempt_at, refreshingSince]);

  const state = deriveHangarState(hangar, refreshingSince);
  const dotTone = hangarDotTone(state);
  const kicker = hangarKicker(state);

  const onRefresh = async () => {
    setRefreshError(null);
    setRefreshingSince(Date.now());
    try {
      await api.refreshHangarNow();
    } catch (err) {
      setRefreshingSince(null);
      const f = friendlyError(err);
      setRefreshError(`${f.title}: ${f.body}`);
    }
  };

  return (
    <TrayCard
      title="Hangar"
      kicker={
        <span
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            gap: 6,
          }}
        >
          <StatusDot tone={dotTone} />
          {kicker}
        </span>
      }
      right={
        <GhostButton
          onClick={onRefresh}
          disabled={state.kind === 'refreshing'}
          style={{ padding: '4px 10px', fontSize: 11 }}
        >
          {state.kind === 'refreshing' ? 'Refreshing…' : 'Refresh now'}
        </GhostButton>
      }
    >
      <HangarBody state={state} ships={hangar.ships_pushed} />
      {refreshError && (
        <p
          style={{
            margin: '8px 0 0',
            fontSize: 12,
            color: 'var(--danger)',
          }}
        >
          Refresh failed: {refreshError}
        </p>
      )}
    </TrayCard>
  );
}

function hangarDotTone(state: HangarState): 'ok' | 'warn' | 'danger' | 'info' | 'dim' {
  switch (state.kind) {
    case 'fresh_success':
      return 'ok';
    case 'stale_success':
      return 'info';
    case 'refreshing':
      return 'info';
    case 'skipped':
      return 'warn';
    case 'error':
      return 'danger';
    case 'never_started':
      return 'dim';
  }
}

function hangarKicker(state: HangarState): string {
  switch (state.kind) {
    case 'never_started':
      return 'not started';
    case 'refreshing':
      return 'fetching from RSI…';
    case 'fresh_success':
      return `✓ ${ageLabel(state.at)}`;
    case 'stale_success':
      return ageLabel(state.at);
    case 'skipped':
      return 'skipped';
    case 'error':
      return 'error';
  }
}

function HangarBody({ state, ships }: { state: HangarState; ships: number }) {
  const shipCount = ships.toLocaleString();

  switch (state.kind) {
    case 'never_started':
      return (
        <p style={{ margin: 0, color: 'var(--fg-dim)', fontSize: 13 }}>
          Pair this device and configure the API URL to start syncing your
          hangar from the RSI website.
        </p>
      );

    case 'refreshing':
      return (
        <p style={{ margin: 0, color: 'var(--fg)', fontSize: 13 }}>
          Fetching the latest hangar snapshot from RSI… last known total{' '}
          <strong style={{ fontVariantNumeric: 'tabular-nums' }}>
            {shipCount}
          </strong>{' '}
          ships.
        </p>
      );

    case 'fresh_success':
      return (
        <p style={{ margin: 0, color: 'var(--fg)', fontSize: 13 }}>
          <span style={{ color: 'var(--ok)' }}>
            Fetched{' '}
            <strong style={{ fontVariantNumeric: 'tabular-nums' }}>
              {shipCount}
            </strong>{' '}
            ships from RSI
          </span>{' '}
          <span style={{ color: 'var(--fg-dim)' }}>
            · {ageLabel(state.at)} ({fmtTime(state.at)})
          </span>
        </p>
      );

    case 'stale_success':
      return (
        <p style={{ margin: 0, color: 'var(--fg)', fontSize: 13 }}>
          Last successful fetch{' '}
          <strong>{ageLabel(state.at)}</strong>{' '}
          <span style={{ color: 'var(--fg-dim)' }}>
            ({fmtTime(state.at)})
          </span>{' '}
          ·{' '}
          <strong style={{ fontVariantNumeric: 'tabular-nums' }}>
            {shipCount}
          </strong>{' '}
          ships pushed
        </p>
      );

    case 'skipped':
      return (
        <p style={{ margin: 0, color: 'var(--warn)', fontSize: 13 }}>
          Skipped: {state.reason}
          {state.at && (
            <span style={{ color: 'var(--fg-dim)' }}>
              {' '}
              · {ageLabel(state.at)}
            </span>
          )}
        </p>
      );

    case 'error':
      return (
        <p style={{ margin: 0, color: 'var(--danger)', fontSize: 13 }}>
          {state.message}
          {state.at && (
            <span style={{ color: 'var(--fg-dim)' }}>
              {' '}
              · {ageLabel(state.at)}
            </span>
          )}
        </p>
      );
  }
}
