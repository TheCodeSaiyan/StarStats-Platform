/**
 * Sessions index for a user — the "See all sessions" drill-down target
 * from the sessions widget. Was MISSING (only the per-session
 * `[session_id]` detail page existed), so the widget's "See all →"
 * link 404'd — and a 404 RSC prefetch/click bails the client navigation
 * back to the dashboard ("loads then reverts"). This is the index.
 *
 * Server component — fetches `/v1/users/{handle}/sessions` with the
 * visitor's bearer token; each row links to
 * `/u/{handle}/sessions/{session_id}` for the per-event timeline.
 * Access control is server-side (401/403 collapse to the same
 * "not available" render so we never leak whether sessions exist).
 */

import Link from 'next/link';
import type { Route } from 'next';
import { ApiCallError, getSessions, type SessionSummary } from '@/lib/api';
import { logger } from '@/lib/logger';
import { getSession } from '@/lib/session';

export const metadata = { title: 'Sessions' };

interface PageProps {
  params: Promise<{ handle: string }>;
}

function fmtDuration(startIso?: string | null, endIso?: string | null): string {
  if (!startIso || !endIso) return '—';
  const a = Date.parse(startIso);
  const b = Date.parse(endIso);
  if (Number.isNaN(a) || Number.isNaN(b) || b <= a) return '—';
  const mins = Math.round((b - a) / 60000);
  const h = Math.floor(mins / 60);
  const m = mins % 60;
  return h > 0 ? (m > 0 ? `${h}h ${m}m` : `${h}h`) : `${m}m`;
}

function fmtDate(iso?: string | null): string {
  if (!iso) return '—';
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return '—';
  return new Date(t).toISOString().slice(0, 16).replace('T', ' ');
}

async function loadSessions(
  bearer: string,
  handle: string,
): Promise<SessionSummary[] | { forbidden: true }> {
  try {
    const res = await getSessions(bearer, handle);
    return res?.sessions ?? [];
  } catch (e) {
    if (
      e instanceof ApiCallError &&
      (e.status === 401 || e.status === 403 || e.status === 404)
    ) {
      return { forbidden: true };
    }
    logger.warn({ err: e, call: 'page.sessions', handle }, 'sessions fetch failed');
    return { forbidden: true };
  }
}

function SessionsUnavailable({ handle }: { handle: string }) {
  return (
    // No `role="main"`: the segment layout's projection owns the single
    // landmark on `#hp-content`.
    <div className="ss-screen-enter" style={{ padding: '8px 2px' }}>
      <h1 style={{ margin: 0, fontSize: 24 }}>Sessions not available</h1>
      <p style={{ margin: '10px 0 0', color: 'var(--fg-muted)' }}>
        <span className="mono">{handle}</span> hasn&apos;t shared their play
        sessions, or you don&apos;t have access.{' '}
        <Link href={(`/u/${encodeURIComponent(handle)}`) as Route}>
          Back to profile
        </Link>
      </p>
    </div>
  );
}

export default async function SessionsIndexPage(props: PageProps) {
  const { handle } = await props.params;

  const session = await getSession();
  if (!session) return <SessionsUnavailable handle={handle} />;

  const result = await loadSessions(session.token, handle);
  if ('forbidden' in result) return <SessionsUnavailable handle={handle} />;

  const sessions = [...result].sort(
    (a, b) => Date.parse(b.started_at ?? '') - Date.parse(a.started_at ?? ''),
  );

  return (
    <div
      className="ss-screen-enter"
      style={{ display: 'flex', flexDirection: 'column', gap: 20 }}
    >
      <header>
        <div className="ss-eyebrow" style={{ marginBottom: 8 }}>
          Sessions
        </div>
        <h1 style={{ margin: 0, fontSize: 32, fontWeight: 600, letterSpacing: '-0.02em' }}>
          <span className="mono">{handle}</span>
          <span style={{ color: 'var(--fg-dim)' }}>{' / '}</span>
          <span style={{ color: 'var(--fg-muted)', fontSize: 24 }}>Sessions</span>
        </h1>
        <p style={{ margin: '8px 0 0', color: 'var(--fg-muted)', fontSize: 14 }}>
          <span style={{ color: 'var(--fg-dim)' }}>
            {sessions.length.toLocaleString()} session
            {sessions.length === 1 ? '' : 's'} recorded.
          </span>
        </p>
        <div style={{ marginTop: 14 }}>
          <Link
            href={(`/u/${encodeURIComponent(handle)}`) as Route}
            className="ss-btn ss-btn--ghost"
            style={{ textDecoration: 'none' }}
          >
            ← Back to profile
          </Link>
        </div>
      </header>

      {sessions.length === 0 ? (
        <p style={{ color: 'var(--fg-muted)' }}>No sessions recorded yet.</p>
      ) : (
        <ul className="hud-readout-list" style={{ gap: 6 }}>
          {sessions.map((s) => (
            <li key={s.id}>
              <Link
                href={
                  (`/u/${encodeURIComponent(handle)}/sessions/${encodeURIComponent(s.id)}`) as Route
                }
                className="hud-readout-row"
                style={{
                  display: 'flex',
                  justifyContent: 'space-between',
                  gap: 12,
                  textDecoration: 'none',
                  color: 'inherit',
                  padding: '8px 10px',
                  border: '1px solid var(--border)',
                  borderRadius: 0,
                }}
              >
                <span>{fmtDate(s.started_at)}</span>
                <span style={{ color: 'var(--fg-muted)' }}>
                  {fmtDuration(s.started_at, s.ended_at)} ·{' '}
                  {(s.event_count ?? 0).toLocaleString()} ev
                </span>
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
